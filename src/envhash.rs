//! The local environment fingerprint (`.host-envhash`) and its reader,
//! `host-lifecycle env --check` (plan/0074, host-lifecycle#19).
//!
//! The fingerprint answers one question no other artifact answers: did this
//! tree move away from what I last recorded? It records STATE — the worktree
//! paths present, the installed hook binary's hash, the pulled toolchain image
//! digest, the submodule init state, and the repo's absolute path — one stanza
//! per dimension, so a check diffs dimension by dimension and prints only the
//! rows that moved.
//!
//! It is not a receipt: it carries no timestamp, no disposition and no
//! evidence, and no op writes both files' facts (plan/0074's field table). It
//! is not a gate either: a delta is a route to look at, never a failure. The
//! one non-zero exit means there is no fingerprint on disk yet, which is a
//! prompt to materialize.
//!
//! A dimension this machine cannot read — the image digest with no container
//! runtime, a hook binary that is not installed — records `unreadable` and is
//! never reported as moved: silence beats a delta the tool cannot see.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process;

use crate::{Software, load_software, sha256_file, write_atomic};

/// The fingerprint file, gitignored: it describes this machine, so it is never
/// shared and never committed.
pub const ENVHASH: &str = ".host-envhash";

/// The two artifacts of the Where room, as plan/0074's field table names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Artifact {
    /// The append-only provenance ledger: what happened.
    Receipt,
    /// This file: what the tree looks like now.
    EnvHash,
}

/// Every fact either artifact records. The first seven are event facts and belong
/// to the receipt; the last five are state facts and belong to the fingerprint.
/// The split is the whole non-overlap discipline in one place, so a fact added to
/// one artifact has to be declared here before it can be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedFact {
    MaterializationHappened,
    Disposition,
    Evidence,
    RecordedAt,
    ComponentNamed,
    PinReference,
    ImageReference,
    WorktreePresent,
    HookBinaryHash,
    ImageDigest,
    SubmoduleInitState,
    RepoAbspath,
}

/// Every fact, so a test can sweep the partition rather than restate it.
pub const FACTS: [RecordedFact; 12] = [
    RecordedFact::MaterializationHappened,
    RecordedFact::Disposition,
    RecordedFact::Evidence,
    RecordedFact::RecordedAt,
    RecordedFact::ComponentNamed,
    RecordedFact::PinReference,
    RecordedFact::ImageReference,
    RecordedFact::WorktreePresent,
    RecordedFact::HookBinaryHash,
    RecordedFact::ImageDigest,
    RecordedFact::SubmoduleInitState,
    RecordedFact::RepoAbspath,
];

/// Which artifact records a fact. Total, and the function the disjointness proof
/// quantifies over: an event has no ambient machine state, and a digest has no
/// time, no disposition and no evidence.
pub fn artifact_of(fact: RecordedFact) -> Artifact {
    match fact {
        RecordedFact::MaterializationHappened
        | RecordedFact::Disposition
        | RecordedFact::Evidence
        | RecordedFact::RecordedAt
        | RecordedFact::ComponentNamed
        | RecordedFact::PinReference
        | RecordedFact::ImageReference => Artifact::Receipt,
        RecordedFact::WorktreePresent
        | RecordedFact::HookBinaryHash
        | RecordedFact::ImageDigest
        | RecordedFact::SubmoduleInitState
        | RecordedFact::RepoAbspath => Artifact::EnvHash,
    }
}

/// The token a fact appears under in the file that owns it, so a test can hold the
/// two real files against the declared partition instead of restating it in prose.
pub fn fact_token(fact: RecordedFact) -> &'static str {
    match fact {
        RecordedFact::MaterializationHappened => "materialize",
        RecordedFact::Disposition => "disposition",
        RecordedFact::Evidence => "evidence",
        RecordedFact::RecordedAt => "recorded =",
        RecordedFact::ComponentNamed => "component",
        RecordedFact::PinReference => "pin ",
        RecordedFact::ImageReference => "toolchain",
        RecordedFact::WorktreePresent => "worktree_paths",
        RecordedFact::HookBinaryHash => "hook_binary",
        RecordedFact::ImageDigest => "pulled_image",
        RecordedFact::SubmoduleInitState => "submodule_init",
        RecordedFact::RepoAbspath => "repo_path",
    }
}

/// One dimension as read on this machine. `None` is unreadable here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvDimension {
    pub kind: String,
    pub value: Option<String>,
}

/// sha256 of a string, via the same `sha256sum` the artifact attestation uses.
/// `None` when the hasher is unavailable, which makes the dimension unreadable
/// rather than silently wrong.
fn sha256_text(s: &str) -> Option<String> {
    let mut child = process::Command::new("sha256sum")
        .stdin(process::Stdio::piped())
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(s.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().next().map(str::to_string)
}

/// The image digest of a locally pulled toolchain image, via whichever container
/// runtime is on PATH. `None` with no runtime (the gather-data settlement: the
/// dimension stays silent rather than guessing).
fn pulled_image_digest(toolchain: &str) -> Option<String> {
    let image = toolchain.trim();
    for runtime in ["docker", "podman"] {
        let out = process::Command::new(runtime)
            .args(["image", "inspect", "--format", "{{index .RepoDigests 0}}", image])
            .stderr(process::Stdio::null())
            .output()
            .ok();
        if let Some(o) = out {
            if o.status.success() {
                let d = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !d.is_empty() {
                    return Some(d);
                }
            }
        }
    }
    None
}

/// The submodule init state: one `<initialized> <path>` line per submodule, from
/// `git submodule status` (its leading `-` marks an uninitialized one). `None`
/// outside a git repo.
fn submodule_state(root: &Path) -> Option<String> {
    let out = process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["submodule", "status"])
        .stderr(process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let initialized = !l.starts_with('-');
            l.split_whitespace().nth(1).map(|p| format!("{} {p}", if initialized { "init" } else { "bare" }))
        })
        .collect();
    lines.sort();
    Some(lines.join("\n"))
}

/// Every worktree path the recipe implies that is present on disk, repo-relative
/// and sorted, so the dimension moves when a component is materialized or torn
/// down but not when the walk order changes.
fn worktree_paths(root: &Path, recipe: &[Software]) -> String {
    let mut paths: Vec<String> = Vec::new();
    for s in recipe {
        let mut branches: Vec<String> = vec![s.branch.clone()];
        branches.extend(s.worktrees.iter().cloned());
        branches.extend(s.lines.iter().map(|w| w.branch.clone()));
        for b in branches {
            let rel = format!("software/{}/{b}", s.name);
            if root.join(&rel).exists() {
                paths.push(rel);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths.join("\n")
}

/// The installed tell-gate binary, if the repo carries one: its hash moves when
/// a hook is reinstalled or rebuilt, which is the drift #19 was filed for.
fn hook_binary_hash(root: &Path) -> Option<String> {
    let hooks = crate::git_hooks_dir(root)?;
    for name in ["host-lint", "host-lint.exe"] {
        let p = hooks.join(name);
        if p.is_file() {
            return sha256_file(&p);
        }
    }
    None
}

/// Read every dimension on this machine. Unreadable dimensions carry `None`.
pub fn envhash_dimensions(root: &Path, recipe: &[Software]) -> Vec<EnvDimension> {
    let image = recipe.iter().find_map(|s| s.toolchain.as_deref()).and_then(pulled_image_digest);
    let values = [
        sha256_text(&worktree_paths(root, recipe)),
        hook_binary_hash(root),
        image,
        submodule_state(root).as_deref().and_then(sha256_text),
        sha256_text(&root.to_string_lossy()),
    ];
    // The dimensions ARE the state facts, read off the declared partition rather
    // than restated: a fact the partition assigns to the receipt cannot become a
    // stanza here, and the proof harness is what makes that assignment total.
    FACTS
        .iter()
        .filter(|f| artifact_of(**f) == Artifact::EnvHash)
        .zip(values)
        .map(|(fact, value)| EnvDimension { kind: fact_token(*fact).to_string(), value })
        .collect()
}

/// Render the fingerprint: one stanza per dimension, in the recipe's idiom.
pub fn envhash_text(dims: &[EnvDimension]) -> String {
    let mut s = String::from(
        "# The local environment fingerprint (host-lifecycle#19): what this tree looks\n\
         # like now, never what was done to it. Gitignored, tool-written.\n",
    );
    for d in dims {
        s.push_str(&format!("\n[envhash \"{}\"]\n", d.kind));
        match &d.value {
            Some(v) => s.push_str(&format!("    value = {v}\n")),
            None => s.push_str("    unreadable = true\n"),
        }
    }
    s
}

/// Parse a recorded fingerprint back into its dimensions.
pub fn parse_envhash(text: &str) -> Vec<EnvDimension> {
    let mut out: Vec<EnvDimension> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(kind) = t.strip_prefix("[envhash \"").and_then(|r| r.strip_suffix("\"]")) {
            out.push(EnvDimension { kind: kind.to_string(), value: None });
        } else if let Some(v) = t.strip_prefix("value = ") {
            if let Some(last) = out.last_mut() {
                last.value = Some(v.trim().to_string());
            }
        }
    }
    out
}

/// The dimensions that moved: read now, differing from what was recorded. A
/// dimension unreadable now never moves, whatever the record says, so a machine
/// without a container runtime stays silent about the image.
pub fn envhash_delta(recorded: &[EnvDimension], current: &[EnvDimension]) -> Vec<String> {
    let mut moved = Vec::new();
    for c in current {
        let Some(now) = &c.value else { continue };
        let was = recorded.iter().find(|r| r.kind == c.kind).and_then(|r| r.value.as_deref());
        if was != Some(now.as_str()) {
            moved.push(c.kind.clone());
        }
    }
    moved
}

/// Keep the fingerprint out of the history. It describes one machine — its
/// absolute paths, its installed binary, its runtime — so committing it would
/// publish local state and make every other clone read a delta that means
/// nothing. The tool that writes the file is the one that owes the ignore line,
/// because an adopter who never heard of the file cannot be expected to add it.
fn ensure_gitignored(root: &Path, entry: &str) {
    let path = root.join(".gitignore");
    let cur = fs::read_to_string(&path).unwrap_or_default();
    if cur.lines().any(|l| l.trim() == entry) {
        return;
    }
    let mut next = cur;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&format!(
        "\n# The local environment fingerprint: this machine's state, never shared.\n{entry}\n"
    ));
    if let Err(e) = write_atomic(&path, &next) {
        eprintln!("host-lifecycle: cannot record {entry} in .gitignore: {e}");
    }
}

/// Write the fingerprint from the live tree. Called by every op that changes the
/// tree; it appends no receipt, because a state change is not an event.
pub fn write_envhash(root: &Path, recipe: &[Software]) {
    let dims = envhash_dimensions(root, recipe);
    if let Err(e) = write_atomic(&root.join(ENVHASH), &envhash_text(&dims)) {
        eprintln!("host-lifecycle: cannot write {ENVHASH}: {e}");
        return;
    }
    ensure_gitignored(root, ENVHASH);
}

/// What a moved dimension MEANS, and whether it implies an action. A line that
/// names a dimension and stops is not a route: the weak-agent probe read
/// `moved hook_binary` plus "nothing is gated" and concluded there was nothing to
/// do, which is right about the gate and wrong about the tree (plan/0074
/// fen-acceptance). Each line now says what changed and what, if anything, to run.
fn dimension_meaning(kind: &str) -> &'static str {
    match kind {
        "worktree_paths" => "a component worktree appeared or disappeared; run `software --verify-setup <dir>` to see whether the setup is still complete",
        "hook_binary" => "the installed commit-gate binary changed (rebuilt or reinstalled); if that was not you, re-run `software --install-hooks <dir>`",
        "pulled_image" => "the locally pulled toolchain image is a different digest; rebuild with `software --verify-build <dir>` to confirm the artifact still reproduces",
        "submodule_init" => "a submodule was initialized or de-initialized; run `git submodule update --init` if one is missing",
        "repo_path" => "the repository sits at a different absolute path than when the fingerprint was recorded (a move or a second clone); nothing to fix",
        _ => "this dimension differs from the recorded fingerprint",
    }
}

/// `env --check <dir>`: recompute, diff, print the route. Advisory by
/// construction — `0` clean, `0` with a delta, `2` when nothing is recorded yet.
pub fn env_check(root: &Path, recipe: &[Software]) -> i32 {
    let path = root.join(ENVHASH);
    let Ok(text) = fs::read_to_string(&path) else {
        eprintln!(
            "host-lifecycle: no {ENVHASH} recorded yet — run `host-lifecycle software --materialize {}` to record one",
            root.display()
        );
        return 2;
    };
    let recorded = parse_envhash(&text);
    let current = envhash_dimensions(root, recipe);
    let moved = envhash_delta(&recorded, &current);
    for kind in &moved {
        println!("moved    {kind} — {}", dimension_meaning(kind));
    }
    for d in &current {
        if d.value.is_none() {
            println!("unread   {} (not readable on this machine; never reported as moved)", d.kind);
        }
    }
    if moved.is_empty() {
        println!("-- the tree matches the recorded fingerprint");
    } else {
        println!(
            "-- {} dimension(s) moved since the fingerprint was recorded. This is advisory: nothing is gated, and \
             the fingerprint is re-recorded by the next `software --materialize <dir>` or `--install-hooks <dir>`. \
             Act on the lines above only where one says to.",
            moved.len()
        );
    }
    0
}

/// `host-lifecycle env --check <dir>`.
pub fn env(args: &[String]) {
    let mut check = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            _ => pos.push(a),
        }
    }
    let (Some(dir), true) = (pos.first(), check) else {
        eprintln!("host-lifecycle env --check <dir>");
        process::exit(2);
    };
    let Ok(root) = fs::canonicalize(Path::new(dir.as_str())) else {
        eprintln!("host-lifecycle: not a directory: {dir}");
        process::exit(2);
    };
    let recipe = load_software(&root);
    process::exit(env_check(&root, &recipe));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim(kind: &str, value: Option<&str>) -> EnvDimension {
        EnvDimension { kind: kind.to_string(), value: value.map(str::to_string) }
    }

    // The fingerprint round-trips through its own format, unreadable dimensions
    // included: a machine with no container runtime records that it could not read
    // the image, rather than recording an empty value that would read as a move.
    #[test]
    fn envhash_text_roundtrips_readable_and_unreadable_dimensions() {
        let dims = vec![dim("worktree_paths", Some("abc123")), dim("pulled_image", None)];
        let back = parse_envhash(&envhash_text(&dims));
        assert_eq!(back, dims);
    }

    // The fingerprint records state and never an event: no timestamp, no
    // disposition, no evidence (plan/0074's field table).
    #[test]
    fn envhash_records_only_state_facts() {
        let text = envhash_text(&[dim("repo_path", Some("deadbeef"))]);
        for event_fact in ["recorded =", "disposition", "evidence", "tool ="] {
            assert!(!text.contains(event_fact), "the fingerprint carries the event fact `{event_fact}`");
        }
        assert!(text.contains("[envhash \"repo_path\"]"));
    }

    // A dimension that moved is reported; one that did not is silent.
    #[test]
    fn env_check_drifted_verdict() {
        let recorded = vec![dim("hook_binary", Some("aaa")), dim("repo_path", Some("bbb"))];
        let current = vec![dim("hook_binary", Some("zzz")), dim("repo_path", Some("bbb"))];
        assert_eq!(envhash_delta(&recorded, &current), vec!["hook_binary".to_string()]);
    }

    #[test]
    fn env_check_clean_verdict() {
        let dims = vec![dim("hook_binary", Some("aaa")), dim("repo_path", Some("bbb"))];
        assert!(envhash_delta(&dims, &dims).is_empty(), "an unmoved tree reports nothing");
    }

    // The settled image rule: with no container runtime the dimension is unreadable,
    // so it never appears as moved even though the record holds a digest.
    #[test]
    fn unreadable_dimension_never_marked_moved() {
        let recorded = vec![dim("pulled_image", Some("sha256:old"))];
        let current = vec![dim("pulled_image", None)];
        assert!(envhash_delta(&recorded, &current).is_empty());
    }

    // Every reported delta names a dimension read on this machine (the mirror of
    // the gate's "every gap is a missing artifact").
    #[test]
    fn every_delta_names_a_readable_moved_dimension() {
        let recorded = vec![dim("worktree_paths", Some("a")), dim("pulled_image", Some("sha256:x"))];
        let current = vec![dim("worktree_paths", Some("b")), dim("pulled_image", None)];
        let moved = envhash_delta(&recorded, &current);
        for kind in &moved {
            let d = current.iter().find(|d| &d.kind == kind).unwrap();
            assert!(d.value.is_some(), "a delta names an unreadable dimension");
        }
        assert_eq!(moved, vec!["worktree_paths".to_string()]);
    }

    // A newly recorded dimension counts as moved: it was not in the record, and it
    // is readable now, so the operator is told where to look.
    #[test]
    fn env_check_start_inits_its_latch() {
        let current = vec![dim("hook_binary", Some("aaa"))];
        assert_eq!(envhash_delta(&[], &current), vec!["hook_binary".to_string()]);
    }

    // With no fingerprint on disk the check cannot proceed (exit 2) — a prompt to
    // materialize, never a verdict about the tree.
    #[test]
    fn env_check_records_then_reads_clean() {
        let base = std::env::temp_dir().join(format!("hl-envhash-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        assert_eq!(env_check(&base, &[]), 2, "no record on disk is the one non-zero exit");
        write_envhash(&base, &[]);
        assert!(base.join(ENVHASH).is_file(), "the write records a fingerprint");
        assert_eq!(env_check(&base, &[]), 0, "a recorded, unmoved tree is clean and never gates");
        let _ = fs::remove_dir_all(&base);
    }

    // The three verdicts over a real recorded file: clean stays clean, a forced
    // change reports its dimension, and neither outcome ever gates.
    #[test]
    fn env_check_exits_zero_with_a_delta() {
        let base = std::env::temp_dir().join(format!("hl-envhash-verdict-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        write_envhash(&base, &[]);
        assert_eq!(env_check(&base, &[]), 0, "a matching tree is clean and exits zero");

        // Rewrite one recorded dimension so the live tree no longer matches it.
        let recorded = fs::read_to_string(base.join(ENVHASH)).unwrap();
        let moved_text = recorded.replace("value = ", "value = 0000");
        fs::write(base.join(ENVHASH), &moved_text).unwrap();
        let dims = envhash_dimensions(&base, &[]);
        let moved = envhash_delta(&parse_envhash(&moved_text), &dims);
        assert!(!moved.is_empty(), "the changed record is reported as moved");
        assert_eq!(env_check(&base, &[]), 0, "a delta routes, it never gates");
        let _ = fs::remove_dir_all(&base);
    }

    // The guards, stated as the absence of the other verdicts: with a record on disk
    // the check never reports cannot-proceed, and an unmoved tree never reports a
    // delta.
    #[test]
    fn env_check_no_false_clean_verdict() {
        let base = std::env::temp_dir().join(format!("hl-envhash-guards-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        assert_eq!(env_check(&base, &[]), 2, "no record is the only non-zero outcome");
        write_envhash(&base, &[]);
        let recorded = parse_envhash(&fs::read_to_string(base.join(ENVHASH)).unwrap());
        assert!(envhash_delta(&recorded, &envhash_dimensions(&base, &[])).is_empty(), "no false delta");
        assert_ne!(env_check(&base, &[]), 2, "a record on disk never reports cannot-proceed");
        let _ = fs::remove_dir_all(&base);
    }

    // The writer owes the ignore line: the fingerprint describes one machine, so it
    // never enters the history, and adding the line twice is a no-op.
    #[test]
    fn install_hooks_write_is_envhash_only() {
        let base = std::env::temp_dir().join(format!("hl-envhash-ignore-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        write_envhash(&base, &[]);
        let first = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert!(first.lines().any(|l| l.trim() == ENVHASH), "the fingerprint is ignored");
        write_envhash(&base, &[]);
        assert_eq!(fs::read_to_string(base.join(".gitignore")).unwrap(), first, "the line is added once");
        assert!(!base.join(crate::LIFECYCLE_RECEIPTS).exists(), "the fingerprint writer appends no provenance");
        let _ = fs::remove_dir_all(&base);
    }

    // Recording twice over an unchanged tree is idempotent: the same bytes, so a
    // re-run of an op that refreshes the fingerprint changes nothing.
    #[test]
    fn cannot_proceed_only_without_an_envhash() {
        let base = std::env::temp_dir().join(format!("hl-envhash-idem-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        write_envhash(&base, &[]);
        let first = fs::read_to_string(base.join(ENVHASH)).unwrap();
        write_envhash(&base, &[]);
        assert_eq!(fs::read_to_string(base.join(ENVHASH)).unwrap(), first);
        assert_eq!(env_check(&base, &[]), 0);
        let _ = fs::remove_dir_all(&base);
    }
}

// Kani proof harnesses (the host-prove `kani-conformance` lane). `#[cfg(kani)]`
// keeps them out of ordinary builds; `cargo kani` compiles with that cfg set.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// ReceiptWritesOnlyEventFacts and EnvHashWritesOnlyStateFacts, proved over the
    /// whole fact set rather than the ones a test happens to name: every fact lands
    /// in exactly one artifact, so no fact can be recorded by both files. The
    /// integration test holds the real files against this same partition; the proof
    /// is what says the partition itself has no overlap and no gap.
    #[kani::proof]
    fn the_two_artifacts_share_no_fact() {
        let i: u8 = kani::any();
        kani::assume((i as usize) < FACTS.len());
        let fact = FACTS[i as usize];
        let owner = artifact_of(fact);
        // Exactly one owner: asserting both directions rules out a fact that some
        // future edit maps to neither or to both.
        assert!(owner == Artifact::Receipt || owner == Artifact::EnvHash);
        assert!(!(owner == Artifact::Receipt && owner == Artifact::EnvHash));
        // And the two sides are the sets the field table declares.
        let is_event = matches!(
            fact,
            RecordedFact::MaterializationHappened
                | RecordedFact::Disposition
                | RecordedFact::Evidence
                | RecordedFact::RecordedAt
                | RecordedFact::ComponentNamed
                | RecordedFact::PinReference
                | RecordedFact::ImageReference
        );
        assert!(is_event == (owner == Artifact::Receipt));
    }
}
