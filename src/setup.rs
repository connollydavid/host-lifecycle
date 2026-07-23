//! The completeness gate, `host-lifecycle software --verify-setup` (plan/0074).
//!
//! `software --check` answers pin-versus-recorded and `env --check` answers
//! drift-from-recorded. Neither notices that this clone's hooks were never
//! installed, that a re-deriver is missing from PATH, or that a skill link was
//! never made: the recorded state can be perfect while the local setup is half
//! done. This gate answers the remaining question, complete-versus-recipe, and
//! it is the question a fresh clone actually fails.
//!
//! **The requirement set comes from the RECIPE, never from the tree.** The
//! recipe holds the closed-world answer: which components exist, which branches
//! each declares, which artifact each records, which component provides the
//! commit gate. The tree is consulted only to answer present-or-absent. An
//! earlier shape derived the requirements by listing directories, and every
//! fail-open it had was the same shape: emptying a directory or deleting a
//! declared line removed the QUESTION instead of answering it "no".
//!
//! It writes nothing at all: no receipt (it verifies, it does not act) and no
//! fingerprint (it reads the recipe, not the recorded digest). Every finding is
//! a required artifact that is absent, so every remedy installs the missing
//! thing; a value that merely moved is `env --check`'s business.
//!
//! Host-role aware: a build this host was never asked to produce (its recipe
//! defers to another attest host) is not required here, so a non-build host does
//! not hazard on an artifact it was never meant to hold.

use std::path::{Path, PathBuf};

use crate::{
    Software, declared_rung_tokens, git_hooks_dir, off_platform, rung_rederiver_problem, sha256_file,
    worktree_dir, worktree_label,
};

/// The classes of local artifact a bootstrapped tree carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementKind {
    SubmodulesInitialized,
    WorktreeMaterialized,
    GateArtifactRecorded,
    HostHookInstalled,
    WorktreeHookInstalled,
    HookBinaryCurrent,
    BuildArtifactPresent,
    RederiverOnPath,
    SkillSourceReadable,
    SkillLinked,
}

impl RequirementKind {
    fn label(&self) -> &'static str {
        match self {
            RequirementKind::SubmodulesInitialized => "submodules initialized",
            RequirementKind::WorktreeMaterialized => "worktree materialized",
            RequirementKind::GateArtifactRecorded => "gate provider records its artifact",
            RequirementKind::HostHookInstalled => "host repository gated",
            RequirementKind::WorktreeHookInstalled => "worktree gated",
            RequirementKind::HookBinaryCurrent => "installed gate matches the built one",
            RequirementKind::BuildArtifactPresent => "build artifact present",
            RequirementKind::RederiverOnPath => "re-deriver runnable",
            RequirementKind::SkillSourceReadable => "skill source readable",
            RequirementKind::SkillLinked => "skill linked",
        }
    }

    /// What to run to install the missing thing, against THIS tree. A remedy an
    /// agent has to complete itself is a remedy that gets pasted with its
    /// placeholder intact, so the root is interpolated rather than named `<dir>`.
    fn remedy(&self, root: &Path, rederiver: Option<&Path>) -> String {
        let dir = root.display();
        match self {
            RequirementKind::SubmodulesInitialized => format!("git -C {dir} submodule update --init"),
            RequirementKind::WorktreeMaterialized => format!("host-lifecycle software --materialize {dir}"),
            RequirementKind::GateArtifactRecorded => {
                "record `artifact = <path> <sha256>` for the component that declares `hooks`; the commit gate installs that binary".to_string()
            }
            RequirementKind::HostHookInstalled
            | RequirementKind::WorktreeHookInstalled
            | RequirementKind::HookBinaryCurrent => {
                format!("host-lifecycle software --install-hooks {dir}")
            }
            RequirementKind::BuildArtifactPresent => {
                format!("build the component in its recorded toolchain: host-lifecycle software --verify-build {dir}")
            }
            RequirementKind::RederiverOnPath => match rederiver {
                // The path form when the recipe carries the re-deriver as a component,
                // the released form when it does not: an adopter who installed the tool
                // the ordinary way must not be told to restructure their recipe.
                Some(w) => format!("cargo install --path {}", w.display()),
                None => "cargo install --git https://github.com/connollydavid/host-prove".to_string(),
            },
            RequirementKind::SkillSourceReadable => format!("host-lifecycle software --materialize {dir}"),
            RequirementKind::SkillLinked => format!("host-lifecycle bootstrap {dir}"),
        }
    }
}

/// One required local artifact, as the recipe implies it for this host.
#[derive(Debug, Clone)]
pub struct Requirement {
    pub kind: RequirementKind,
    /// What the requirement is about: a component, a worktree path, a skill name.
    pub target: String,
    pub required: bool,
    pub present: bool,
    /// Why this host does not carry the requirement, printed on the `n-a` line.
    /// Two different conditions make a build artifact unrequired, and a line that
    /// blames the wrong one teaches the reader something false about their tree.
    pub note: String,
}

impl Requirement {
    /// A gap: required here, and not there. Nothing else is a gap — a value that
    /// moved is drift, and drift is another concern's question.
    pub fn is_gap(&self) -> bool {
        self.required && !self.present
    }
}

fn need(kind: RequirementKind, target: String, present: bool) -> Requirement {
    Requirement { kind, target, required: true, present, note: String::new() }
}

/// Whether this host is the one the recipe expects to produce a given build: an
/// entry deferring to another attest host is not this host's obligation.
fn built_here(attest_host: Option<&str>) -> bool {
    attest_host.is_none_or(|h| h == std::env::consts::OS)
}

/// Every worktree the recipe DECLARES for a component, as (label, path): the
/// canonical branch, each bare `worktrees =` branch, and each explicit
/// `worktree =` line that applies to this platform. Enumerated from the recipe,
/// so deleting a materialized worktree removes the artifact, never the question.
pub fn declared_worktrees(root: &Path, s: &Software) -> Vec<(String, PathBuf)> {
    let mut out = vec![(worktree_label(&s.name, &s.branch), worktree_dir(root, &s.name, &s.branch))];
    for b in &s.worktrees {
        out.push((worktree_label(&s.name, b), worktree_dir(root, &s.name, b)));
    }
    for w in &s.lines {
        if off_platform(&w.host) {
            continue;
        }
        out.push((worktree_label(&s.name, &w.branch), crate::line_target(root, &s.name, w)));
    }
    out
}

/// Whether a path is really a git worktree, not merely a directory that exists.
/// An emptied worktree and one whose gitdir link points nowhere both pass
/// `is_dir()`, and both are exactly the half-made state this gate exists to
/// report.
fn is_git_worktree(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    crate::git_out(path, &["rev-parse", "--show-toplevel"])
        .and_then(|top| std::fs::canonicalize(top).ok())
        .zip(std::fs::canonicalize(path).ok())
        .is_some_and(|(top, here)| top == here)
}

/// The recorded artifact for a component, through the same accessor every other
/// reader uses: the per-platform view whose attest-host matches, else the flat
/// fields. Reading only the flat fields made the gate blind to every adopter on
/// the `[build …]` form.
fn recorded_artifact(s: &Software) -> Option<(String, String)> {
    s.builds_view()
        .into_iter()
        .find(|b| built_here(b.attest_host) && b.artifact.is_some())
        .and_then(|b| b.artifact.cloned())
}

/// Whether a hooks directory carries a live gate: both hook names present AND
/// executable, and the binary beside them. A hook without its executable bit is
/// ignored by git, so a filename check alone reports a gate that never runs —
/// which is this plan's founding failure, one layer down.
fn hooks_installed(hooks_dir: &Path, bin_name: &str) -> bool {
    ["pre-commit", "commit-msg"].iter().all(|f| is_executable(&hooks_dir.join(f)))
        && hooks_dir.join(bin_name).is_file()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && std::fs::metadata(path).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// Windows has no executable bit; presence is all the filesystem can say.
#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Whether the installed gate binary is the one the worktree currently holds. The
/// installer deliberately accepts a local build, so this compares against what
/// `--install-hooks` would install NOW rather than against the canonical hash: a
/// stale or hand-edited gate is the case worth reporting.
fn hook_binary_current(hooks_dir: &Path, bin_name: &str, built: &Path) -> bool {
    match (sha256_file(&hooks_dir.join(bin_name)), sha256_file(built)) {
        (Some(installed), Some(built)) => installed == built,
        // The built artifact is absent: its own requirement covers that, and this
        // one has nothing to compare, so it does not also fire.
        _ => true,
    }
}

/// Read the recipe and the live tree into the requirement set this host carries.
pub fn setup_requirements(root: &Path, recipe: &[Software]) -> Vec<Requirement> {
    let mut reqs: Vec<Requirement> = Vec::new();

    // The orchestrator inits the submodules first, so the gate owes that step a
    // class; otherwise a tree with every submodule absent verifies complete.
    for (path, initialized) in crate::bootstrap::submodule_states(root) {
        reqs.push(need(RequirementKind::SubmodulesInitialized, path.clone(), initialized));
    }

    let mut rung_declared = false;
    for s in recipe {
        for (label, path) in declared_worktrees(root, s) {
            reqs.push(need(RequirementKind::WorktreeMaterialized, label, is_git_worktree(&path)));
        }
        let canonical = worktree_dir(root, &s.name, &s.branch);
        if !declared_rung_tokens(&canonical).is_empty() {
            rung_declared = true;
        }
        for b in s.builds_view() {
            let Some((path, _)) = b.artifact else { continue };
            // Two conditions, both about role, and each with its own reason: the
            // artifact is required LOCALLY only when something local consumes it
            // (today the commit gate, which installs the hooks binary from the
            // worktree) and only when this host is the one the recipe expects to
            // build it.
            let consumed = s.hooks.is_some();
            let note = if !consumed {
                "no local consumer: this artifact is attested in its toolchain, not installed here".to_string()
            } else {
                format!("built on {}, not here", b.attest_host.unwrap_or("another host"))
            };
            reqs.push(Requirement {
                kind: RequirementKind::BuildArtifactPresent,
                target: match b.platform {
                    Some(p) => format!("{} [{p}]", s.name),
                    None => s.name.clone(),
                },
                required: consumed && built_here(b.attest_host),
                present: canonical.join(path).is_file(),
                note,
            });
        }
    }

    // Skills: enumerated from the sources the orchestrator writes, and a source
    // that cannot be READ is its own gap. An unreadable directory used to delete
    // every skill requirement under it, so gutting a submodule turned required
    // links into no requirement at all.
    let (skills, unreadable) = crate::bootstrap::skill_sources_checked(root, recipe);
    for src in unreadable {
        reqs.push(need(RequirementKind::SkillSourceReadable, src.display().to_string(), false));
    }
    for (skill, src) in skills {
        let dest = root.join(".claude").join("skills").join(&skill);
        // Resolving to the SOURCE, not merely existing: a plain directory or a link
        // into another checkout satisfied `exists()` while gating nothing.
        let linked = match (std::fs::canonicalize(&dest), std::fs::canonicalize(&src)) {
            (Ok(d), Ok(s)) => d == s,
            _ => false,
        };
        reqs.push(need(RequirementKind::SkillLinked, skill, linked));
    }

    // The gate providers: a component declaring a hooks script gates every commit
    // surface, so its absence anywhere is a gap (plan/0074, Bug A).
    for s in recipe.iter().filter(|s| s.hooks.is_some()) {
        // A hooks script with no recorded artifact is a recipe defect, not a reason
        // to stop asking: the installer refuses that component outright, so the gate
        // must say so rather than fall silent about every hook it implies.
        let Some((art_path, _)) = recorded_artifact(s) else {
            reqs.push(need(RequirementKind::GateArtifactRecorded, s.name.clone(), false));
            continue;
        };
        let bin_name = Path::new(&art_path).file_name().unwrap_or_default().to_string_lossy().to_string();
        let built = worktree_dir(root, &s.name, &s.branch).join(&art_path);

        let mut surfaces: Vec<(RequirementKind, String, Option<PathBuf>)> =
            vec![(RequirementKind::HostHookInstalled, s.name.clone(), git_hooks_dir(root))];
        for c in recipe {
            for (label, path) in declared_worktrees(root, c) {
                surfaces.push((RequirementKind::WorktreeHookInstalled, label, git_hooks_dir(&path)));
            }
        }
        let mut seen: Vec<PathBuf> = Vec::new();
        for (kind, target, hooks) in surfaces {
            // An unresolvable hooks directory is an ungated surface, which is exactly
            // the state worth reporting: absent, never omitted.
            let Some(hooks) = hooks else {
                reqs.push(need(kind, target, false));
                continue;
            };
            if seen.contains(&hooks) {
                continue; // git shares one hooks dir across a store's worktrees
            }
            seen.push(hooks.clone());
            reqs.push(need(kind, target.clone(), hooks_installed(&hooks, &bin_name)));
            reqs.push(need(
                RequirementKind::HookBinaryCurrent,
                target,
                hook_binary_current(&hooks, &bin_name, &built),
            ));
        }
    }

    if rung_declared {
        // The token the recipe actually declares, not a literal from this project's
        // rung set: the probe is keyed on what the specs asked for.
        let token = recipe
            .iter()
            .find_map(|s| declared_rung_tokens(&worktree_dir(root, &s.name, &s.branch)).first().copied())
            .unwrap_or("kani:");
        reqs.push(need(
            RequirementKind::RederiverOnPath,
            "the shared re-deriver".to_string(),
            rung_rederiver_problem(token).is_none(),
        ));
    }
    reqs
}

/// The gate: print one line per requirement, HAZARD on every gap, and settle the
/// verdict. `0` complete, `1` hazarded, matching `software --check`'s convention.
pub fn verify_setup(root: &Path, recipe: &[Software]) -> i32 {
    let reqs = setup_requirements(root, recipe);
    let rederiver = crate::bootstrap::rederiver_worktree(root, recipe);
    let mut gaps = 0usize;
    for r in &reqs {
        if r.is_gap() {
            println!(
                "HAZARD   {} — {} is missing; run: {}",
                r.target,
                r.kind.label(),
                r.kind.remedy(root, rederiver.as_deref())
            );
            gaps += 1;
        } else if !r.required {
            println!("n-a      {} — {} ({})", r.target, r.kind.label(), r.note);
        } else {
            println!("ok       {} — {}", r.target, r.kind.label());
        }
    }
    if gaps > 0 {
        println!("-- setup INCOMPLETE: {gaps} required local artifact(s) missing; install them and re-run");
        return 1;
    }
    println!("-- setup complete: every artifact the recipe requires of this host is present");
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn req(kind: RequirementKind, target: &str, required: bool, present: bool) -> Requirement {
        Requirement { kind, target: target.to_string(), required, present, note: String::new() }
    }

    // The gate's only detector: a required artifact that is not present.
    #[test]
    fn missing_required_artifact_gaps() {
        assert!(req(RequirementKind::HostHookInstalled, "host-lint", true, false).is_gap());
    }

    // Present, or never required here, is never a gap.
    #[test]
    fn present_or_unrequired_artifact_never_gaps() {
        assert!(!req(RequirementKind::HostHookInstalled, "host-lint", true, true).is_gap());
        assert!(!req(RequirementKind::BuildArtifactPresent, "host-lint", false, false).is_gap());
    }

    // A gap names the requirement it came from, so the line says which worktree is
    // ungated rather than that "a hook" is missing.
    #[test]
    fn worktree_hook_gap_names_the_worktree() {
        let r = req(RequirementKind::WorktreeHookInstalled, "software/host-lint/main", true, false);
        assert!(r.is_gap());
        assert_eq!(r.target, "software/host-lint/main");
        let remedy = r.kind.remedy(Path::new("/tmp/tree"), None);
        assert!(remedy.contains("--install-hooks"), "the remedy installs the missing thing");
        assert!(remedy.contains("/tmp/tree"), "and it names this tree, never a `<dir>` to paste");
    }

    // Host-role awareness: a build the recipe defers to another attest host is not
    // required here, so its absence is not a gap.
    #[test]
    fn non_build_host_never_gaps_on_the_artifact() {
        assert!(built_here(None), "no attest host declared means any host builds it");
        assert!(built_here(Some(std::env::consts::OS)));
        assert!(!built_here(Some("plan9")));
        let elsewhere = Requirement {
            kind: RequirementKind::BuildArtifactPresent,
            target: "host-lint [aarch64]".to_string(),
            required: built_here(Some("plan9")),
            present: false,
            note: "built on plan9, not here".to_string(),
        };
        assert!(!elsewhere.is_gap());
    }

    // Every gap is a missing required artifact — never a value that moved. This is
    // the completeness gate's half of the non-overlap with `env --check`.
    #[test]
    fn every_gap_is_a_missing_required_artifact() {
        let reqs = [
            req(RequirementKind::WorktreeMaterialized, "a", true, true),
            req(RequirementKind::SkillLinked, "b", true, false),
            req(RequirementKind::BuildArtifactPresent, "c", false, false),
        ];
        for r in reqs.iter().filter(|r| r.is_gap()) {
            assert!(r.required && !r.present);
        }
        assert_eq!(reqs.iter().filter(|r| r.is_gap()).count(), 1);
    }

    /// A recipe whose component declares the commit gate, with a materialized
    /// worktree: the shape every gate defect hid behind. No test passed one of these
    /// into `setup_requirements` before, so the whole host-and-worktree requirement
    /// generator ran uncovered, and two fail-open paths lived in it.
    fn gate_provider_fixture(base: &Path, artifact: bool) -> Vec<Software> {
        let wt = base.join("software").join("gate").join("main");
        fs::create_dir_all(&wt).unwrap();
        vec![Software {
            name: "gate".into(),
            url: "u".into(),
            pin: "p".into(),
            branch: "main".into(),
            worktrees: vec![],
            lines: vec![],
            build: None,
            toolchain: None,
            deploy: None,
            artifact: artifact.then(|| ("bin/gate".to_string(), "sha".to_string())),
            repro_exempt: None,
            hooks: Some("hooks-script".into()),
            deps_bundle: None,
            builds: vec![],
        }]
    }

    // The gate asks for a hook on the host repository AND on every materialized
    // worktree, and an unresolvable hooks directory reads as ungated rather than as
    // no requirement at all. Fail-open here is how a half-made tree passed.
    #[test]
    fn setup_requires_a_hook_on_every_commit_surface() {
        let base = std::env::temp_dir().join(format!("hl-setup-cover-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let recipe = gate_provider_fixture(&base, true);

        let reqs = setup_requirements(&base, &recipe);
        let host_hooks: Vec<&Requirement> =
            reqs.iter().filter(|r| r.kind == RequirementKind::HostHookInstalled).collect();
        let wt_hooks: Vec<&Requirement> =
            reqs.iter().filter(|r| r.kind == RequirementKind::WorktreeHookInstalled).collect();
        assert_eq!(host_hooks.len(), 1, "the host repository is a commit surface");
        assert_eq!(wt_hooks.len(), 1, "so is every materialized worktree");
        assert!(host_hooks[0].is_gap() && wt_hooks[0].is_gap(), "neither is gated in a bare fixture");
        // The line must NAME the ungated worktree: blanking every target left the
        // invariant green while each hazard read "  — worktree gated is missing".
        assert!(
            wt_hooks[0].target.contains("gate") && wt_hooks[0].target.contains("main"),
            "the worktree hook gap names its worktree, got `{}`",
            wt_hooks[0].target
        );
        assert_eq!(verify_setup(&base, &recipe), 1, "and the run says so");

        // Host-role awareness, through the real requirement builder: a build the
        // recipe defers to another attest host is not required here. Deleting the
        // `built_here` conjunct left every prior test green.
        let mut elsewhere = gate_provider_fixture(&base, true);
        elsewhere[0].builds = vec![crate::PlatformBuild {
            platform: "aarch64".into(),
            build: Some("cargo build".into()),
            toolchain: None,
            deploy: None,
            artifact: Some(("bin/gate".into(), "sha".into())),
            repro_exempt: None,
            attest_host: Some("plan9".into()),
        }];
        let reqs = setup_requirements(&base, &elsewhere);
        let art = reqs
            .iter()
            .find(|r| r.kind == RequirementKind::BuildArtifactPresent)
            .expect("the artifact requirement exists");
        assert!(!art.required, "a build attested on another host is not required here");
        assert!(!art.is_gap(), "so its absence is not a gap");
        assert!(art.note.contains("plan9"), "and the line says which host builds it");

        // A hooks-declaring component with no recorded artifact is a recipe defect,
        // not a reason to stop asking: the gate names it rather than falling silent
        // about every hook it implies.
        let no_artifact = gate_provider_fixture(&base, false);
        let reqs = setup_requirements(&base, &no_artifact);
        assert!(
            reqs.iter().any(|r| r.kind == RequirementKind::GateArtifactRecorded && r.is_gap()),
            "a gate provider with no artifact is a gap, not a silence"
        );
        assert_eq!(verify_setup(&base, &no_artifact), 1, "and it never reads as complete");
        let _ = fs::remove_dir_all(&base);
    }

    // A hooks gap is detected per commit surface, and a complete setup therefore
    // implies every materialized worktree is gated (the coverage invariant). The
    // check is not filename presence: a hook without its executable bit is ignored
    // by git, so a tree of inert hooks would otherwise read as gated.
    #[test]
    fn complete_setup_implies_every_worktree_hooked() {
        let base = std::env::temp_dir().join(format!("hl-setup-hooks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let hooks = base.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        assert!(!hooks_installed(&hooks, "host-lint"), "an empty hooks dir is not gated");
        for f in ["pre-commit", "commit-msg", "host-lint"] {
            fs::write(hooks.join(f), "x").unwrap();
        }
        assert!(!hooks_installed(&hooks, "host-lint"), "present but not executable is not a gate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for f in ["pre-commit", "commit-msg"] {
                fs::set_permissions(hooks.join(f), fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        assert!(hooks_installed(&hooks, "host-lint"), "executable hooks plus their binary are a gate");
        fs::remove_file(hooks.join("host-lint")).unwrap();
        assert!(!hooks_installed(&hooks, "host-lint"), "the dispatch script without its binary is not a gate");

        // And the installed binary must be the one the worktree holds: a stale copy
        // gates commits against yesterday's rules.
        let built = base.join("built");
        fs::write(&built, "BINARY-v2").unwrap();
        fs::write(hooks.join("host-lint"), "BINARY-v1").unwrap();
        assert!(!hook_binary_current(&hooks, "host-lint", &built), "a stale installed gate is reported");
        fs::write(hooks.join("host-lint"), "BINARY-v2").unwrap();
        assert!(hook_binary_current(&hooks, "host-lint", &built));
        let _ = fs::remove_dir_all(&base);
    }

    // One missing artifact gates the setup: the run reports incomplete however many
    // other requirements are satisfied.
    #[test]
    fn one_gap_blocks_a_complete_verdict() {
        let base = std::env::temp_dir().join(format!("hl-setup-verdict-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let recipe = [Software {
            name: "absent".into(),
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
        }];
        // Nothing is materialized, so the worktree requirement gaps and the verdict
        // is hazarded.
        assert_eq!(verify_setup(&base, &recipe), 1);
        let reqs = setup_requirements(&base, &recipe);
        assert_eq!(reqs.iter().filter(|r| r.is_gap()).count(), 1, "one cause, one gap, no cascade");
        // An empty recipe requires nothing of this host, so it is complete.
        assert_eq!(verify_setup(&base, &[]), 0);
        let _ = fs::remove_dir_all(&base);
    }

    // The verdict counts gaps and nothing else: a satisfied requirement, and one this
    // host was never asked for, both leave the run complete.
    #[test]
    fn verify_setup_no_false_hazard() {
        let reqs = [
            req(RequirementKind::WorktreeMaterialized, "a", true, true),
            req(RequirementKind::BuildArtifactPresent, "b", false, false),
            req(RequirementKind::SkillLinked, "c", true, true),
        ];
        assert_eq!(reqs.iter().filter(|r| r.is_gap()).count(), 0, "nothing missing, nothing to report");
        let with_gap = [req(RequirementKind::RederiverOnPath, "host-prove", true, false)];
        assert_eq!(with_gap.iter().filter(|r| r.is_gap()).count(), 1, "one missing artifact, one gap");
    }

    // A materialized worktree with a skills dir owes a resolving link; a dangling one
    // reads as absent, because a link that does not resolve gates nothing.
    #[test]
    fn gap_names_its_requirement() {
        let base = std::env::temp_dir().join(format!("hl-setup-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("comp").join("main");
        fs::create_dir_all(wt.join("skills").join("tend")).unwrap();
        let recipe = [Software {
            name: "comp".into(),
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
        }];
        let reqs = setup_requirements(&base, &recipe);
        let skill = reqs.iter().find(|r| r.kind == RequirementKind::SkillLinked).expect("skill requirement");
        assert_eq!(skill.target, "tend");
        assert!(skill.is_gap(), "an unlinked skill is a gap");
        let _ = fs::remove_dir_all(&base);
    }
}
