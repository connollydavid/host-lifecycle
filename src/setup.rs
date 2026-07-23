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

use std::fs;
use std::path::Path;

use crate::{Software, declared_rung_tokens, git_hooks_dir, rung_rederiver_problem, worktree_dir};

/// The classes of local artifact a bootstrapped tree carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementKind {
    WorktreeMaterialized,
    HostHookInstalled,
    WorktreeHookInstalled,
    BuildArtifactPresent,
    RederiverOnPath,
    SkillLinked,
}

impl RequirementKind {
    fn label(&self) -> &'static str {
        match self {
            RequirementKind::WorktreeMaterialized => "worktree materialized",
            RequirementKind::HostHookInstalled => "host repository gated",
            RequirementKind::WorktreeHookInstalled => "worktree gated",
            RequirementKind::BuildArtifactPresent => "build artifact present",
            RequirementKind::RederiverOnPath => "re-deriver runnable",
            RequirementKind::SkillLinked => "skill linked",
        }
    }

    /// What to run to install the missing thing. A gate whose remedy an agent has
    /// to infer is a gate that gets bypassed.
    fn remedy(&self) -> &'static str {
        match self {
            RequirementKind::WorktreeMaterialized => "host-lifecycle software --materialize <dir>",
            RequirementKind::HostHookInstalled | RequirementKind::WorktreeHookInstalled => {
                "host-lifecycle software --install-hooks <dir>"
            }
            RequirementKind::BuildArtifactPresent => "build the component in its recorded toolchain",
            RequirementKind::RederiverOnPath => "install host-prove on PATH (cargo install --path software/host-prove/main --root ~/.local)",
            RequirementKind::SkillLinked => "regenerate the skill links (link-skills.sh)",
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

/// The skill directories a materialized worktree offers, by name.
fn worktree_skills(worktree: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(worktree.join("skills")) else { return Vec::new() };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// Read the recipe and the live tree into the requirement set this host carries.
pub fn setup_requirements(root: &Path, recipe: &[Software]) -> Vec<Requirement> {
    let mut reqs: Vec<Requirement> = Vec::new();
    let mut rung_declared = false;
    for s in recipe {
        let worktree = worktree_dir(root, &s.name, &s.branch);
        let materialized = worktree.is_dir();
        reqs.push(Requirement {
            kind: RequirementKind::WorktreeMaterialized,
            target: s.name.clone(),
            required: true,
            present: materialized,
        });
        if !materialized {
            // Every remaining requirement for this component reads the worktree, so
            // there is nothing further to say until it exists: one gap, one remedy,
            // rather than a cascade that buries the first cause.
            continue;
        }
        for b in s.builds_view() {
            if let Some((path, _)) = b.artifact {
                reqs.push(Requirement {
                    kind: RequirementKind::BuildArtifactPresent,
                    target: match b.platform {
                        Some(p) => format!("{} [{p}]", s.name),
                        None => s.name.clone(),
                    },
                    // Two conditions, both about role. The artifact is required LOCALLY
                    // only when something local consumes it — today that is the commit
                    // gate, which installs the hooks binary from the worktree — and only
                    // when this host is the one the recipe expects to build it. A
                    // component whose artifact exists to be attested in a container, or
                    // on another attest host, is not this tree's obligation: requiring it
                    // would hazard every clone that has not run the heavy build lane.
                    required: s.hooks.is_some() && built_here(b.attest_host),
                    present: worktree.join(path).is_file(),
                });
            }
        }
        if !declared_rung_tokens(&worktree).is_empty() {
            rung_declared = true;
        }
        for skill in worktree_skills(&worktree) {
            let linked = root.join(".claude").join("skills").join(&skill).exists();
            reqs.push(Requirement {
                kind: RequirementKind::SkillLinked,
                target: skill,
                required: true,
                present: linked,
            });
        }
    }
    // The gate providers: a component declaring a hooks script gates every commit
    // surface, so its absence anywhere is a gap (plan/0074, Bug A).
    for s in recipe.iter().filter(|s| s.hooks.is_some()) {
        let Some((art_path, _)) = &s.artifact else { continue };
        let bin_name = Path::new(art_path).file_name().unwrap_or_default().to_string_lossy().to_string();
        if let Some(hooks) = git_hooks_dir(root) {
            reqs.push(Requirement {
                kind: RequirementKind::HostHookInstalled,
                target: s.name.clone(),
                required: true,
                present: hooks_installed(&hooks, &bin_name),
            });
        }
        for c in recipe {
            let worktree = worktree_dir(root, &c.name, &c.branch);
            if !worktree.is_dir() {
                continue;
            }
            let Some(hooks) = git_hooks_dir(&worktree) else { continue };
            reqs.push(Requirement {
                kind: RequirementKind::WorktreeHookInstalled,
                target: format!("software/{}/{}", c.name, c.branch),
                required: true,
                present: hooks_installed(&hooks, &bin_name),
            });
        }
    }
    if rung_declared {
        reqs.push(Requirement {
            kind: RequirementKind::RederiverOnPath,
            target: "host-prove".to_string(),
            required: true,
            present: rung_rederiver_problem("kani:").is_none(),
        });
    }
    reqs
}

/// The gate: print one line per requirement, HAZARD on every gap, and settle the
/// verdict. `0` complete, `1` hazarded, matching `software --check`'s convention.
pub fn verify_setup(root: &Path, recipe: &[Software]) -> i32 {
    let reqs = setup_requirements(root, recipe);
    let mut gaps = 0usize;
    for r in &reqs {
        if r.is_gap() {
            println!("HAZARD   {} — {} is missing; run: {}", r.target, r.kind.label(), r.kind.remedy());
            gaps += 1;
        } else if !r.required {
            println!("n-a      {} — {} (not this host's role)", r.target, r.kind.label());
        } else {
            println!("ok       {} — {}", r.target, r.kind.label());
        }
    }
    if gaps > 0 {
        eprintln!("-- setup INCOMPLETE: {gaps} required local artifact(s) missing; install them and re-run");
        return 1;
    }
    println!("-- setup complete: every artifact the recipe requires of this host is present");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(kind: RequirementKind, target: &str, required: bool, present: bool) -> Requirement {
        Requirement { kind, target: target.to_string(), required, present }
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
        assert!(r.kind.remedy().contains("--install-hooks"), "the remedy installs the missing thing");
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

    // A materialized worktree with a skills dir owes a resolving link; a dangling one
    // reads as absent, because a link that does not resolve gates nothing.
    #[test]
    fn gap_names_its_requirement() {
        let base = std::env::temp_dir().join(format!("hl-setup-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("comp").join("main");
        fs::create_dir_all(wt.join("skills").join("tend")).unwrap();
        assert_eq!(worktree_skills(&wt), vec!["tend".to_string()]);
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
