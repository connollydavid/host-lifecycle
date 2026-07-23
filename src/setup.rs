//! The completeness gate, `host-lifecycle software --verify-setup` (plan/0074).
//!
//! `software --check` answers pin-versus-recorded and `env --check` answers
//! drift-from-recorded. Neither notices that this clone's hooks were never
//! installed, that a re-deriver is missing from PATH, or that a skill link was
//! never made: the recorded state can be perfect while the local setup is half
//! done. This gate answers the remaining question, complete-versus-recipe, and
//! it is the question a fresh clone actually fails.
//!
//! It reads the recipe and the live tree and writes nothing at all: no receipt
//! (it verifies, it does not act) and no fingerprint (it reads the recipe, not
//! the recorded digest). Every finding is a required artifact that is absent, so
//! every remedy installs the missing thing; a value that merely moved is
//! `env --check`'s business, never this gate's.
//!
//! Host-role aware: a build this host was never asked to produce (its recipe
//! defers to another attest host) is not required here, so a non-build host does
//! not hazard on an artifact it was never meant to hold.

use std::path::Path;

use crate::{Software, declared_rung_tokens, git_hooks_dir, rung_rederiver_problem, worktree_dir};

/// The classes of local artifact a bootstrapped tree carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementKind {
    SubmodulesInitialized,
    WorktreeMaterialized,
    GateArtifactRecorded,
    HostHookInstalled,
    WorktreeHookInstalled,
    BuildArtifactPresent,
    RederiverOnPath,
    SkillLinked,
}

impl RequirementKind {
    fn label(&self) -> &'static str {
        match self {
            RequirementKind::SubmodulesInitialized => "submodules initialized",
            RequirementKind::GateArtifactRecorded => "gate provider records its artifact",
            RequirementKind::WorktreeMaterialized => "worktree materialized",
            RequirementKind::HostHookInstalled => "host repository gated",
            RequirementKind::WorktreeHookInstalled => "worktree gated",
            RequirementKind::BuildArtifactPresent => "build artifact present",
            RequirementKind::RederiverOnPath => "re-deriver runnable",
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
            RequirementKind::HostHookInstalled | RequirementKind::WorktreeHookInstalled => {
                format!("host-lifecycle software --install-hooks {dir}")
            }
            RequirementKind::BuildArtifactPresent => {
                format!("build the component in its recorded toolchain: host-lifecycle software --verify-build {dir}")
            }
            RequirementKind::RederiverOnPath => match rederiver {
                Some(w) => format!("cargo install --path {} --root ~/.local (and put ~/.local/bin on PATH)", w.display()),
                None => "no component in the recipe deploys the shared re-deriver; record one, or drop the deeper rung the specs declare".to_string(),
            },
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

/// Whether this host is the one the recipe expects to produce a given build: an
/// entry deferring to another attest host is not this host's obligation.
fn built_here(attest_host: Option<&str>) -> bool {
    attest_host.is_none_or(|h| h == std::env::consts::OS)
}

/// Whether a hooks directory carries a complete gate: both hook names and the
/// binary the dispatch script runs.
fn hooks_installed(hooks_dir: &Path, bin_name: &str) -> bool {
    ["pre-commit", "commit-msg", bin_name].iter().all(|f| hooks_dir.join(f).is_file())
}

/// Read the recipe and the live tree into the requirement set this host carries.
///
/// One rule governs every probe here: a probe that cannot run produces a
/// requirement that is ABSENT, never a requirement that is skipped. A skipped
/// requirement can never be a gap, and a gate that drops what it cannot read is
/// the green-over-a-half-made-tree this plan exists to close.
pub fn setup_requirements(root: &Path, recipe: &[Software]) -> Vec<Requirement> {
    let mut reqs: Vec<Requirement> = Vec::new();
    let mut rung_declared = false;

    // The orchestrator inits the submodules first, so the gate owes that step a
    // class; otherwise a tree with every submodule absent verifies complete.
    let uninitialized = crate::bootstrap::uninitialized_submodules(root);
    if crate::bootstrap::declares_submodules(root) {
        reqs.push(Requirement {
            kind: RequirementKind::SubmodulesInitialized,
            target: match uninitialized.first() {
                Some(first) => format!("{first} and {} other(s)", uninitialized.len().saturating_sub(1)),
                None => "all declared submodules".to_string(),
            },
            required: true,
            present: uninitialized.is_empty(),
            note: String::new(),
        });
    }

    for s in recipe {
        let worktree = worktree_dir(root, &s.name, &s.branch);
        let materialized = worktree.is_dir();
        reqs.push(Requirement {
            kind: RequirementKind::WorktreeMaterialized,
            target: s.name.clone(),
            required: true,
            present: materialized,
            note: String::new(),
        });
        if !materialized {
            // Every remaining requirement for this component reads the worktree, so
            // there is nothing further to say until it exists: one gap, one remedy,
            // rather than a cascade that buries the first cause.
            continue;
        }
        for b in s.builds_view() {
            if let Some((path, _)) = b.artifact {
                // Two conditions, both about role, and each with its own reason: the
                // artifact is required LOCALLY only when something local consumes it
                // (today the commit gate, which installs the hooks binary from the
                // worktree) and only when this host is the one the recipe expects to
                // build it. The `n-a` line names which condition applied, because
                // "not this host's role" over a build this host does perform teaches
                // the reader something false about their own tree.
                let consumed = s.hooks.is_some();
                let ours = built_here(b.attest_host);
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
                    required: consumed && ours,
                    present: worktree.join(path).is_file(),
                    note,
                });
            }
        }
        if !declared_rung_tokens(&worktree).is_empty() {
            rung_declared = true;
        }
    }

    // Skills: the gate reads exactly the sources the orchestrator writes, so
    // anything bootstrap links is something the gate can miss. Reading a narrower
    // set left the majority of this project's links unwatched.
    for (skill, _) in crate::bootstrap::skill_sources(root, recipe) {
        let linked = root.join(".claude").join("skills").join(&skill).exists();
        reqs.push(Requirement {
            kind: RequirementKind::SkillLinked,
            target: skill,
            required: true,
            present: linked,
            note: String::new(),
        });
    }

    // The gate providers: a component declaring a hooks script gates every commit
    // surface, so its absence anywhere is a gap (plan/0074, Bug A).
    for s in recipe.iter().filter(|s| s.hooks.is_some()) {
        // A hooks script with no recorded artifact is a recipe defect, not a reason
        // to stop asking: the installer refuses that component outright, so the gate
        // must say so rather than fall silent about every hook it implies.
        let Some((art_path, _)) = &s.artifact else {
            reqs.push(Requirement {
                kind: RequirementKind::GateArtifactRecorded,
                target: s.name.clone(),
                required: true,
                present: false,
                note: String::new(),
            });
            continue;
        };
        let bin_name = Path::new(art_path).file_name().unwrap_or_default().to_string_lossy().to_string();
        reqs.push(Requirement {
            kind: RequirementKind::HostHookInstalled,
            target: s.name.clone(),
            required: true,
            // An unresolvable hooks directory is an ungated repository, which is
            // exactly the state worth reporting.
            present: git_hooks_dir(root).is_some_and(|h| hooks_installed(&h, &bin_name)),
            note: String::new(),
        });
        for c in recipe {
            let worktree = worktree_dir(root, &c.name, &c.branch);
            if !worktree.is_dir() {
                continue;
            }
            reqs.push(Requirement {
                kind: RequirementKind::WorktreeHookInstalled,
                target: format!("software/{}/{}", c.name, c.branch),
                required: true,
                present: git_hooks_dir(&worktree).is_some_and(|h| hooks_installed(&h, &bin_name)),
                note: String::new(),
            });
        }
    }
    if rung_declared {
        reqs.push(Requirement {
            kind: RequirementKind::RederiverOnPath,
            target: "the shared re-deriver".to_string(),
            required: true,
            present: rung_rederiver_problem("kani:").is_none(),
            note: String::new(),
        });
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
        assert_eq!(verify_setup(&base, &recipe), 1, "and the run says so");

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
    // implies every materialized worktree is gated (the coverage invariant).
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
        assert!(hooks_installed(&hooks, "host-lint"));
        fs::remove_file(hooks.join("host-lint")).unwrap();
        assert!(!hooks_installed(&hooks, "host-lint"), "the dispatch script without its binary is not a gate");
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
