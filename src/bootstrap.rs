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
        .filter_map(|l| {
            let t = l.trim();
            // `path = x` and `path=x` are both legal git config.
            t.strip_prefix("path")
                .map(str::trim_start)
                .and_then(|r| r.strip_prefix('='))
                .map(|p| root.join(p.trim()))
        })
        .collect()
}

/// Every declared submodule and whether it is usable here, as (path, ok). Read
/// from git, which is authoritative: `path = x` and `path=x` are both legal in
/// `.gitmodules`, and a parser that misses one reports "no submodules" over a
/// tree full of them. A submodule git calls initialized but whose working tree
/// was gutted is NOT ok: an empty directory offers nothing and every requirement
/// sourced from it would otherwise vanish.
pub fn submodule_states(root: &Path) -> Vec<(String, bool)> {
    let git = submodule_status(root);
    let declared = submodule_paths(root);
    let mut out: Vec<(String, bool)> = git
        .iter()
        .map(|(init, path)| {
            let dir = root.join(path);
            let populated = dir.read_dir().map(|mut d| d.any(|e| e.map(|e| e.file_name() != ".git").unwrap_or(false))).unwrap_or(false);
            (path.clone(), *init && populated)
        })
        .collect();
    // `.gitmodules` declares one git does not report (an un-staged gitlink, or no
    // git at all): the disagreement is itself a gap, never a reason to drop the
    // class.
    for p in declared {
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().to_string();
        if !out.iter().any(|(path, _)| *path == rel) {
            out.push((rel, false));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn submodule_status(root: &Path) -> Vec<(bool, String)> {
    let Ok(out) = process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["submodule", "status"])
        .stderr(process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(|p| (!l.starts_with('-'), p.to_string())))
        .collect()
}

/// Every skill a materialized worktree or an initialized submodule offers, as
/// (name, source dir). This is the generic form of a project's link-skills
/// script: the sources are what the recipe materialized, not a hardcoded list.
pub fn skill_sources(root: &Path, recipe: &[Software]) -> Vec<(String, PathBuf)> {
    skill_sources_checked(root, recipe).0
}

/// Every skill a materialized worktree or an initialized submodule offers, as
/// (name, source dir), PLUS the sources that could not be enumerated. An
/// unreadable source is not "no skills": it is a question this machine cannot
/// answer, and answering it "none" deleted requirements that existed a moment
/// earlier.
pub fn skill_sources_checked(root: &Path, recipe: &[Software]) -> (Vec<(String, PathBuf)>, Vec<PathBuf>) {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut unreadable: Vec<PathBuf> = Vec::new();
    // (component name from the RECIPE, directory): a name derived from the path
    // leaf reads a non-`main` canonical branch as the component's name.
    let mut dirs: Vec<(String, PathBuf)> = recipe
        .iter()
        .map(|s| (s.name.clone(), worktree_dir(root, &s.name, &s.branch)))
        .filter(|(_, d)| d.is_dir())
        .collect();
    for p in submodule_paths(root).into_iter().filter(|p| p.is_dir()) {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        dirs.push((name, p));
    }
    for (component, dir) in dirs {
        // A component that ships one root SKILL.md is linked under its own name.
        // Missing this form meant the orchestrator produced a strictly smaller set of
        // links than the script it replaces, and the gate could not see the difference.
        if dir.join("SKILL.md").is_file() {
            out.push((component.clone(), dir.clone()));
        }
        let skills_dir = dir.join("skills");
        let rd = match fs::read_dir(&skills_dir) {
            Ok(rd) => rd,
            // Absent is legitimate (most components offer no skills); present but
            // unreadable is a gap the caller must hear about.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                unreadable.push(skills_dir);
                continue;
            }
        };
        let mut found: Vec<(String, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
            .collect();
        found.sort();
        out.extend(found);
    }
    out.sort();
    out.dedup();
    (out, unreadable)
}

/// The components whose artifact this tree needs locally: the commit gate installs
/// the binary from the worktree, so a missing one blocks the hook install. Other
/// artifacts are attested in their recorded toolchain, never built by bootstrap.
fn artifact_owed(root: &Path, recipe: &[Software]) -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    for s in recipe.iter().filter(|s| s.hooks.is_some()) {
        let worktree = worktree_dir(root, &s.name, &s.branch);
        if !worktree.is_dir() {
            continue;
        }
        // Through `builds_view()`, the same accessor the completeness gate reads: a
        // component using the per-platform `[build]` form has no flat `artifact` or
        // `build` field, and reading only the flat fields made the orchestrator and
        // the gate disagree about whether there was anything to build.
        for b in s.builds_view() {
            let Some((path, _)) = b.artifact else { continue };
            if b.attest_host.is_some_and(|h| h != std::env::consts::OS) {
                continue;
            }
            if !worktree.join(path).is_file() {
                // A recipe with no `build` line cannot say how to produce it, which
                // is worth reporting rather than skipping.
                let how = b.build.unwrap_or("no `build =` recorded: the recipe cannot say how to produce it");
                out.push((s.name.clone(), how.to_string(), worktree.clone()));
            }
        }
    }
    out
}

/// The worktree of the component that carries the shared re-deriver, whatever the
/// recipe calls it. host-prove is the one driver every deeper rung goes through
/// (call/0018), so the BINARY name is generic; the component name and its path are
/// not, and matching a literal against the `deploy` line (which names the deployed
/// worktree, not the component) skipped the step for every adopter but this one.
pub fn rederiver_worktree(root: &Path, recipe: &[Software]) -> Option<PathBuf> {
    recipe
        .iter()
        .find(|s| s.name == "host-prove" || s.deploy.as_deref() == Some("host-prove"))
        .map(|s| worktree_dir(root, &s.name, &s.branch))
        .filter(|d| d.is_dir())
}

/// The re-deriver install this tree owes: a declared rung, a driver that does not
/// run here, and a component to install it from.
fn rederiver_owed(root: &Path, recipe: &[Software]) -> Option<PathBuf> {
    let declared = recipe.iter().any(|s| {
        let wt = worktree_dir(root, &s.name, &s.branch);
        wt.is_dir() && !declared_rung_tokens(&wt).is_empty()
    });
    if !declared || rung_rederiver_problem("kani:").is_none() {
        return None;
    }
    rederiver_worktree(root, recipe)
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
    // Satisfaction is the GATE's judgement, read fresh. The orchestrator used to
    // carry its own copy of each predicate, and the two drifted: a component with
    // an artifact and no build line made the gate hazard while the step read
    // satisfied, so re-running never converged.
    let unmet = |kinds: &[crate::setup::RequirementKind]| -> usize {
        crate::setup::setup_requirements(root, recipe)
            .iter()
            .filter(|r| kinds.contains(&r.kind) && r.is_gap())
            .count()
    };
    let (detail, satisfied) = match kind {
        StepKind::InitSubmodules => {
            let n = unmet(&[crate::setup::RequirementKind::SubmodulesInitialized]);
            (format!("{n} uninitialized"), n == 0)
        }
        StepKind::Materialize => {
            let n = unmet(&[
                crate::setup::RequirementKind::WorktreeMaterialized,
                crate::setup::RequirementKind::SkillSourceReadable,
            ]);
            (format!("{} component(s), {n} worktree(s) missing", recipe.len()), n == 0)
        }
        StepKind::LinkSkills => {
            let n = unmet(&[crate::setup::RequirementKind::SkillLinked]);
            (format!("{} skill(s) offered, {n} unlinked", skill_sources(root, recipe).len()), n == 0)
        }
        StepKind::BuildArtifact => {
            let owed = artifact_owed(root, recipe);
            (
                owed.iter().map(|(n, _, _)| n.clone()).collect::<Vec<_>>().join(", "),
                unmet(&[crate::setup::RequirementKind::BuildArtifactPresent]) == 0,
            )
        }
        // The hook install is a copy: idempotent by construction and cheap, so it runs
        // every time rather than being predicted. The gate is what says it landed.
        StepKind::InstallHooks => ("host repository and every materialized worktree".to_string(), false),
        StepKind::InstallRederiver => (
            "the shared re-deriver".to_string(),
            unmet(&[crate::setup::RequirementKind::RederiverOnPath]) == 0,
        ),
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
    let sources = skill_sources(root, recipe);
    // Prune first: a component dropped from the recipe leaves a link into nothing,
    // and a dangling link trips every tree walker. The project script this
    // replaces cleared them the same way.
    if let Ok(rd) = fs::read_dir(&dest_root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let is_link = fs::symlink_metadata(e.path()).map(|m| m.file_type().is_symlink()).unwrap_or(false);
            if is_link && (!e.path().exists() || !sources.iter().any(|(n, _)| *n == name)) {
                let _ = fs::remove_file(e.path());
                println!("prune    .claude/skills/{name} (no longer offered, or dangling)");
            }
        }
    }
    // Generated, never tracked (call/0005): the tool that makes them owes the
    // ignore line, exactly as the fingerprint's writer does.
    crate::envhash::ensure_gitignored(root, "/.claude/skills/*");
    let mut ok = true;
    for (name, src) in sources {
        let dest = dest_root.join(&name);
        if dest.exists() {
            // Something is there; whether it is the RIGHT thing is the question. A
            // link resolving into another checkout (a copied tree) or an operator's
            // own directory would otherwise read as satisfied forever.
            let same = matches!(
                (fs::canonicalize(&dest), fs::canonicalize(&src)),
                (Ok(ref d), Ok(ref s)) if d == s
            );
            if !same {
                println!("conflict .claude/skills/{name} exists and does not resolve to {}", src.display());
                ok = false;
            }
            continue;
        }
        // A stale link that no longer resolves is replaced rather than kept: a
        // dangling link gates nothing and trips every tree walker.
        let _ = fs::remove_file(&dest);
        // Relative, matching what a project's own link script writes: an absolute
        // link survives a COPY of the tree by pointing back at the original, which
        // resolves, so nothing reports it while the skills belong to another
        // checkout.
        let target = pathdiff(&dest_root, &src);
        if let Err(e) = crate::make_handle(&dest, &target) {
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
                // Bootstrap does NOT run the recorded build. That recipe is written
                // for the digest-pinned toolchain container, where the vendored deps
                // are staged and the target is installed; shelling it into the ambient
                // toolchain either fails opaquely on a fresh clone or, worse, succeeds
                // and installs a binary that is not the canonical one. The step reports
                // what is owed and where to run it, and the gate reports the gap.
                for (name, build, worktree) in artifact_owed(&root, &recipe) {
                    println!("owed     {name} artifact is absent; it is built in the recorded toolchain, not here");
                    println!("         reproduce it: host-lifecycle software --verify-build {}", root.display());
                    println!("         or build it locally in {}: {build}", worktree.display());
                }
            }
            StepKind::InstallHooks => {
                // The non-exiting form: a hook that cannot be installed (its binary is
                // not built yet) is reported and the run continues to the gate, which
                // is what states the verdict. An orchestrator that dies here would
                // report nothing about the rest of the tree.
                let (installed, failed) = crate::install_hooks(&root, &recipe);
                println!("hooks    {installed} target(s) gated, {failed} could not be");
                // The op this step stands in for refreshes the fingerprint, because
                // the gate binary's hash just moved. Skipping it left every freshly
                // bootstrapped tree reporting a false drift on the one dimension this
                // work was founded on.
                crate::envhash::write_envhash(&root, &recipe);
            }
            StepKind::InstallRederiver => {
                let owed = rederiver_owed(&root, &recipe);
                if owed.is_none() {
                    println!("         no component in the recipe carries the re-deriver; the gate names how to install it");
                }
                if let Some(worktree) = owed {
                    println!("install  host-prove from {}", worktree.display());
                    // cargo's own root, which is on PATH for anyone who has cargo —
                    // and bootstrap already needs cargo to run this step at all. A
                    // `--root ~/.local` install lands somewhere the gate then reports
                    // as missing on every run.
                    let ok = process::Command::new("cargo")
                        .args(["install", "--path"])
                        .arg(&worktree)
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

/// `src` expressed relative to `from`, falling back to the absolute path when the
/// two share no root (an off-tree store, a different drive).
fn pathdiff(from: &Path, src: &Path) -> PathBuf {
    let (Ok(from), Ok(src)) = (fs::canonicalize(from), fs::canonicalize(src)) else {
        return src.to_path_buf();
    };
    let mut up = PathBuf::new();
    let mut base = from.as_path();
    loop {
        if let Ok(rest) = src.strip_prefix(base) {
            return up.join(rest);
        }
        match base.parent() {
            Some(p) => {
                up.push("..");
                base = p;
            }
            None => return src,
        }
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

    // Idempotence: the steps a run performs are the ones the GATE reports unmet, so
    // over a tree the gate calls complete only the two unconditional steps run (the
    // hook copy, idempotent by construction, and the gate itself). A bare directory
    // is not a materialized worktree, so this fixture still owes materialize.
    #[test]
    fn bootstrap_completes_after_hooks() {
        let base = std::env::temp_dir().join(format!("hl-boot-idem-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("software").join("demo").join("main")).unwrap();
        let steps = plan_steps(&base, &[comp("demo")]);
        let performed: Vec<&StepKind> = steps.iter().filter(|s| !s.satisfied).map(|s| &s.kind).collect();
        assert!(
            performed.contains(&&StepKind::Materialize),
            "a directory that is not a git worktree is not materialized"
        );
        assert!(performed.contains(&&StepKind::InstallHooks) && performed.contains(&&StepKind::VerifySetup));
        assert!(!performed.contains(&&StepKind::InitSubmodules), "a tree with no submodules owes none");
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
