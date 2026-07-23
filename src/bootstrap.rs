//! The fresh-clone orchestrator, `host-lifecycle bootstrap <dir>` (plan/0074).
//!
//! The setup sequence used to live as prose an agent hand-executed in order:
//! seed the tool, init the submodules, materialize, link the skills, build the
//! gating binary, install the hooks, install the re-derivers. Prose is not a
//! gate, so a step was skipped and the tree sat half made with nothing to say
//! so. This subcommand runs the sequence instead, driven by the recipe rather
//! than by anything specific to one project.
//!
//! It makes state and verifies none of it: every step defers its recording to
//! the op that performs it (the materialize receipt, the fingerprint), and the
//! last step hands the verdict to the completeness gate, so the orchestrator
//! never certifies itself.
//!
//! Every step is a no-op when its precondition already holds, so a second run
//! over a complete tree changes nothing and still ends in the gate.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use crate::{Software, declared_rung_tokens, load_software, rung_rederiver_problem, worktree_dir};

/// One planned step. `satisfied` is read from the live tree before the run, so a
/// step already done is planned, reported, and not performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: StepKind,
    pub detail: String,
    pub satisfied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepKind {
    InitSubmodules,
    Materialize,
    LinkSkills,
    BuildArtifact,
    InstallHooks,
    InstallRederiver,
    VerifySetup,
}

impl StepKind {
    fn label(&self) -> &'static str {
        match self {
            StepKind::InitSubmodules => "init submodules",
            StepKind::Materialize => "materialize the Where room",
            StepKind::LinkSkills => "link the skills",
            StepKind::BuildArtifact => "build the gating artifact",
            StepKind::InstallHooks => "install the commit hooks",
            StepKind::InstallRederiver => "install the re-deriver on PATH",
            StepKind::VerifySetup => "verify the setup is complete",
        }
    }
}

/// Submodule paths declared in `.gitmodules`, and whether each is populated. An
/// empty directory is an uninitialized submodule, which is the state
/// `git submodule update --init` exists to fix.
fn submodule_paths(root: &Path) -> Vec<PathBuf> {
    let text = fs::read_to_string(root.join(".gitmodules")).unwrap_or_default();
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("path = ").map(|p| root.join(p.trim())))
        .collect()
}

fn submodules_populated(root: &Path) -> bool {
    submodule_paths(root)
        .iter()
        .all(|p| p.read_dir().map(|mut d| d.next().is_some()).unwrap_or(false))
}

/// Every skill a materialized worktree or an initialized submodule offers, as
/// (name, source dir). This is the generic form of a project's link-skills
/// script: the sources are what the recipe materialized, not a hardcoded list.
pub fn skill_sources(root: &Path, recipe: &[Software]) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut dirs: Vec<PathBuf> = recipe
        .iter()
        .map(|s| worktree_dir(root, &s.name, &s.branch))
        .filter(|d| d.is_dir())
        .collect();
    dirs.extend(submodule_paths(root).into_iter().filter(|p| p.is_dir()));
    for dir in dirs {
        let Ok(rd) = fs::read_dir(dir.join("skills")) else { continue };
        let mut found: Vec<(String, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
            .collect();
        found.sort();
        out.extend(found);
    }
    out
}

/// Whether every offered skill resolves under `.claude/skills/`.
fn skills_linked(root: &Path, recipe: &[Software]) -> bool {
    skill_sources(root, recipe)
        .iter()
        .all(|(name, _)| root.join(".claude").join("skills").join(name).exists())
}

/// The components whose artifact this tree needs locally: the commit gate installs
/// the binary from the worktree, so a missing one blocks the hook install. Other
/// artifacts are attested in their recorded toolchain, never built by bootstrap.
fn artifact_owed(root: &Path, recipe: &[Software]) -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    for s in recipe.iter().filter(|s| s.hooks.is_some()) {
        let (Some((path, _)), Some(build)) = (&s.artifact, &s.build) else { continue };
        let worktree = worktree_dir(root, &s.name, &s.branch);
        if worktree.is_dir() && !worktree.join(path).is_file() {
            out.push((s.name.clone(), build.clone(), worktree));
        }
    }
    out
}

/// The component that carries the shared re-deriver, when a declared rung needs
/// one and it does not run here.
fn rederiver_owed(root: &Path, recipe: &[Software]) -> Option<PathBuf> {
    let declared = recipe.iter().any(|s| {
        let wt = worktree_dir(root, &s.name, &s.branch);
        wt.is_dir() && !declared_rung_tokens(&wt).is_empty()
    });
    if !declared || rung_rederiver_problem("kani:").is_none() {
        return None;
    }
    recipe
        .iter()
        .find(|s| s.deploy.as_deref() == Some("host-prove"))
        .map(|s| worktree_dir(root, &s.name, &s.branch))
        .filter(|d| d.is_dir())
}

/// The sequence, in order. The steps are fixed; what varies is which of them the
/// live tree already satisfies.
pub const SEQUENCE: [StepKind; 7] = [
    StepKind::InitSubmodules,
    StepKind::Materialize,
    StepKind::LinkSkills,
    StepKind::BuildArtifact,
    StepKind::InstallHooks,
    StepKind::InstallRederiver,
    StepKind::VerifySetup,
];

/// Read one step against the live tree: satisfied, and what it is about. Read
/// immediately before the step runs, never once up front — each step changes the
/// state the next one reads, so a plan fixed in advance would skip work its own
/// earlier steps created (materializing a worktree is what makes its skills
/// visible to link).
pub fn read_step(root: &Path, recipe: &[Software], kind: StepKind) -> Step {
    let (detail, satisfied) = match kind {
        StepKind::InitSubmodules => (
            format!("{} declared", submodule_paths(root).len()),
            submodules_populated(root),
        ),
        StepKind::Materialize => (
            format!("{} component(s)", recipe.len()),
            recipe.iter().all(|s| worktree_dir(root, &s.name, &s.branch).is_dir()),
        ),
        StepKind::LinkSkills => (
            format!("{} skill(s) offered", skill_sources(root, recipe).len()),
            skills_linked(root, recipe),
        ),
        StepKind::BuildArtifact => {
            let owed = artifact_owed(root, recipe);
            (
                owed.iter().map(|(n, _, _)| n.clone()).collect::<Vec<_>>().join(", "),
                owed.is_empty(),
            )
        }
        // The hook install is a copy: idempotent by construction and cheap, so it runs
        // every time rather than being predicted. The gate is what says it landed.
        StepKind::InstallHooks => ("host repository and every materialized worktree".to_string(), false),
        StepKind::InstallRederiver => ("host-prove".to_string(), rederiver_owed(root, recipe).is_none()),
        StepKind::VerifySetup => ("the completeness gate".to_string(), false),
    };
    Step { kind, detail, satisfied }
}

/// The whole sequence read against the tree as it stands. The runner re-reads each
/// step as it reaches it, so this exists for the tests that pin the derivation:
/// which steps a given tree implies, and which of them a complete tree skips.
#[cfg(test)]
pub fn plan_steps(root: &Path, recipe: &[Software]) -> Vec<Step> {
    SEQUENCE.iter().map(|k| read_step(root, recipe, k.clone())).collect()
}

/// Run a command in a directory, reporting what it runs. Bootstrap performs real
/// work, so it says what it is about to do before it does it.
fn run(dir: &Path, program: &str, args: &[&str]) -> bool {
    println!("run      {program} {} (in {})", args.join(" "), dir.display());
    process::Command::new(program)
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn link_skills(root: &Path, recipe: &[Software]) -> bool {
    let dest_root = root.join(".claude").join("skills");
    if let Err(e) = fs::create_dir_all(&dest_root) {
        eprintln!("host-lifecycle: cannot create {}: {e}", dest_root.display());
        return false;
    }
    let mut ok = true;
    for (name, src) in skill_sources(root, recipe) {
        let dest = dest_root.join(&name);
        if dest.exists() {
            continue;
        }
        // A stale link that no longer resolves is replaced rather than kept: a
        // dangling link gates nothing and trips every tree walker.
        let _ = fs::remove_file(&dest);
        if let Err(e) = crate::make_handle(&dest, &src) {
            eprintln!("host-lifecycle: cannot link skill {name}: {e}");
            ok = false;
        } else {
            println!("link     .claude/skills/{name}");
        }
    }
    ok
}

/// `host-lifecycle bootstrap <dir>`: run the sequence, then hand the verdict to
/// the completeness gate. The exit code is the gate's, so a bootstrap that made
/// every state it could still reports incomplete when something is missing.
pub fn bootstrap(args: &[String]) {
    let Some(dir) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("host-lifecycle bootstrap <dir>");
        process::exit(2);
    };
    let Ok(root) = fs::canonicalize(Path::new(dir.as_str())) else {
        eprintln!("host-lifecycle: not a directory: {dir}");
        process::exit(2);
    };
    let recipe = load_software(&root);
    if recipe.is_empty() {
        eprintln!("host-lifecycle: no software recipe in {} — nothing to bootstrap", root.display());
        process::exit(2);
    }
    for kind in SEQUENCE {
        let step = read_step(&root, &recipe, kind);
        if step.satisfied {
            println!("skip     {} ({})", step.kind.label(), step.detail);
            continue;
        }
        println!("step     {} ({})", step.kind.label(), step.detail);
        match step.kind {
            StepKind::InitSubmodules => {
                if !run(&root, "git", &["submodule", "update", "--init"]) {
                    eprintln!("host-lifecycle: submodule init failed; the rest of the sequence needs it");
                    process::exit(1);
                }
            }
            StepKind::Materialize => crate::software_materialize(&root, &recipe, false),
            StepKind::LinkSkills => {
                if !link_skills(&root, &recipe) {
                    eprintln!("host-lifecycle: could not link every skill");
                    process::exit(1);
                }
            }
            StepKind::BuildArtifact => {
                for (name, build, worktree) in artifact_owed(&root, &recipe) {
                    println!("build    {name}: {build}");
                    let ok = process::Command::new("sh")
                        .arg("-c")
                        .arg(&build)
                        .current_dir(&worktree)
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if !ok {
                        eprintln!("host-lifecycle: the recorded build for {name} failed; the commit gate needs its binary");
                        process::exit(1);
                    }
                }
            }
            StepKind::InstallHooks => crate::software_install_hooks(&root, &recipe),
            StepKind::InstallRederiver => {
                if let Some(worktree) = rederiver_owed(&root, &recipe) {
                    println!("install  host-prove from {}", worktree.display());
                    let ok = process::Command::new("cargo")
                        .args(["install", "--path"])
                        .arg(&worktree)
                        .args(["--root", &home_local(&root)])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if !ok {
                        eprintln!("host-lifecycle: could not install the re-deriver; the gate will report it");
                    }
                }
            }
            StepKind::VerifySetup => process::exit(crate::setup::verify_setup(&root, &recipe)),
        }
    }
}

/// Where a PATH tool is installed: `~/.local`, the location the spine documents,
/// falling back to the project root when there is no home directory to speak of.
fn home_local(root: &Path) -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => format!("{h}/.local"),
        _ => root.join(".local").to_string_lossy().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(name: &str) -> Software {
        Software {
            name: name.into(),
            url: "u".into(),
            pin: "p".into(),
            branch: "main".into(),
            worktrees: vec![],
            lines: vec![],
            build: None,
            toolchain: None,
            deploy: None,
            artifact: None,
            repro_exempt: None,
            hooks: None,
            deps_bundle: None,
            builds: vec![],
        }
    }

    // The plan is derived from the recipe and the live tree: nothing materialized
    // means the materialize step is planned, and the gate is always last.
    #[test]
    fn bootstrap_start_inits_its_latch() {
        let base = std::env::temp_dir().join(format!("hl-boot-plan-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let steps = plan_steps(&base, &[comp("demo")]);
        let materialize = steps.iter().find(|s| s.kind == StepKind::Materialize).unwrap();
        assert!(!materialize.satisfied, "an unmaterialized tree plans the materialize step");
        assert_eq!(steps.last().unwrap().kind, StepKind::VerifySetup, "the gate is the last step");
        let _ = fs::remove_dir_all(&base);
    }

    // Idempotence: over a tree whose state already holds, every predicted step reads
    // satisfied, so a second run performs nothing. The two unconditional steps are
    // the hook copy (idempotent by construction) and the gate (a read).
    #[test]
    fn bootstrap_completes_after_hooks() {
        let base = std::env::temp_dir().join(format!("hl-boot-idem-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("software").join("demo").join("main")).unwrap();
        let steps = plan_steps(&base, &[comp("demo")]);
        let performed: Vec<&StepKind> = steps.iter().filter(|s| !s.satisfied).map(|s| &s.kind).collect();
        assert_eq!(performed, vec![&StepKind::InstallHooks, &StepKind::VerifySetup]);
        let _ = fs::remove_dir_all(&base);
    }

    // A worktree that offers a skill owes a link; once the link resolves, the step is
    // satisfied and the second run skips it.
    #[test]
    fn bootstrap_latches_its_hooks_step() {
        let base = std::env::temp_dir().join(format!("hl-boot-skills-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("demo").join("main");
        fs::create_dir_all(wt.join("skills").join("tend")).unwrap();
        let recipe = [comp("demo")];
        assert_eq!(skill_sources(&base, &recipe).len(), 1);
        let plan_before = plan_steps(&base, &recipe);
        assert!(!plan_before.iter().find(|s| s.kind == StepKind::LinkSkills).unwrap().satisfied);
        assert!(link_skills(&base, &recipe));
        assert!(base.join(".claude/skills/tend").exists(), "the link resolves to the worktree's skill");
        let plan_after = plan_steps(&base, &recipe);
        assert!(plan_after.iter().find(|s| s.kind == StepKind::LinkSkills).unwrap().satisfied);
        let _ = fs::remove_dir_all(&base);
    }

    // An artifact is owed only when the commit gate consumes it: a component with no
    // hooks script is attested in its toolchain, never built by bootstrap.
    #[test]
    fn bootstrap_owes_only_the_gate_providers_artifact() {
        let base = std::env::temp_dir().join(format!("hl-boot-build-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("gate").join("main");
        fs::create_dir_all(&wt).unwrap();
        let mut gate = comp("gate");
        gate.artifact = Some(("bin/gate".into(), "sha".into()));
        gate.build = Some("cargo build --release".into());
        let mut plain = comp("plain");
        plain.artifact = Some(("bin/plain".into(), "sha".into()));
        plain.build = Some("cargo build --release".into());
        fs::create_dir_all(base.join("software").join("plain").join("main")).unwrap();
        assert!(artifact_owed(&base, &[gate.clone(), plain.clone()]).is_empty(), "no hooks script, nothing owed");
        gate.hooks = Some("pre-commit".into());
        let owed = artifact_owed(&base, &[gate.clone(), plain]);
        assert_eq!(owed.len(), 1, "only the gate provider's artifact is owed locally");
        assert_eq!(owed[0].0, "gate");
        fs::create_dir_all(wt.join("bin")).unwrap();
        fs::write(wt.join("bin/gate"), "x").unwrap();
        assert!(artifact_owed(&base, &[gate]).is_empty(), "a present artifact is not owed again");
        let _ = fs::remove_dir_all(&base);
    }
}
