//! The outcomes that END a run, exercised through the real binary (plan/0074).
//!
//! A process that exits cannot be observed from inside its own test: an aborted
//! materialize, the advisory environment check, the completeness gate's verdict
//! and the orchestrator's final hand-off all terminate the process, and their
//! exit code IS the contract each one publishes. So these run the built binary as
//! a subprocess and read the code and the output an operator would see.
//!
//! They live here rather than beside the code because cargo guarantees the
//! binary is built for an integration test; a unit test that spawned it could
//! silently exercise a stale build, which is exactly the trap this suite exists
//! to catch elsewhere.

use std::fs;
use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_host-lifecycle");
const RECEIPTS: &str = ".host-lifecycle-receipts";
const ENVHASH: &str = ".host-envhash";

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(BIN).args(args).output().expect("host-lifecycle runs");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git {args:?} failed in {}", dir.display());
}

fn fixture(name: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("hl-exit-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    base
}

/// A source repository with one commit, to materialize from.
fn seed_source(base: &Path) -> (std::path::PathBuf, String) {
    let src = base.join("src");
    fs::create_dir_all(&src).unwrap();
    git(&src, &["init", "-q", "-b", "main"]);
    git(&src, &["config", "user.email", "t@t"]);
    git(&src, &["config", "user.name", "t"]);
    fs::write(src.join("readme.txt"), "seed").unwrap();
    git(&src, &["add", "-A"]);
    git(&src, &["commit", "-qm", "seed"]);
    let out = Command::new("git").arg("-C").arg(&src).args(["rev-parse", "HEAD"]).output().unwrap();
    let pin = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (src, pin)
}

// A materialize that cannot clone never reaches realized: it fails closed and
// leaves no receipt, because a receipt records an event that happened.
#[test]
fn materialize_run_aborts_on_failure() {
    let base = fixture("abort");
    fs::write(
        base.join(".host-software"),
        "[software \"ghost\"]\n\turl = /nonexistent/never/here.git\n\tpin = 0000000000000000000000000000000000000000\n",
    )
    .unwrap();
    let (code, _) = run(&["software", "--materialize", &base.to_string_lossy()]);
    assert_eq!(code, 2, "a clone that cannot run fails closed");
    assert!(!base.join(RECEIPTS).exists(), "an aborted run appends no provenance");
    let _ = fs::remove_dir_all(&base);
}

// The same, with a component that DID realize before the failing one: a receipt
// records a run that realized what it was asked to, so a run that aborted leaves
// none — not even for the components it got through first.
#[test]
fn materialize_abort_leaves_no_receipt_for_earlier_components() {
    let base = fixture("abort-partial");
    let (src, pin) = seed_source(&base);
    let host = base.join("host");
    fs::create_dir_all(&host).unwrap();
    fs::write(
        host.join(".host-software"),
        format!(
            "[software \"good\"]\n\turl = {}\n\tpin = {pin}\n\n[software \"ghost\"]\n\turl = /nonexistent/never/here.git\n\tpin = 0000000000000000000000000000000000000000\n",
            src.to_string_lossy()
        ),
    )
    .unwrap();
    let (code, text) = run(&["software", "--materialize", &host.to_string_lossy()]);
    assert_eq!(code, 2, "the run aborts: {text}");
    assert!(host.join("software").join("good").join("main").is_dir(), "the first component did realize");
    assert!(
        !host.join(RECEIPTS).exists(),
        "and no provenance survives the abort: {}",
        fs::read_to_string(host.join(RECEIPTS)).unwrap_or_default()
    );
    assert!(host.join(ENVHASH).is_file(), "the tree changed, so the fingerprint is refreshed");
    let _ = fs::remove_dir_all(&base);
}

// A materialize that realized worktrees records the event once and refreshes the
// fingerprint at the same call site.
#[test]
fn materialize_run_reaches_realized() {
    let base = fixture("realized");
    let (src, pin) = seed_source(&base);
    let host = base.join("host");
    fs::create_dir_all(&host).unwrap();
    fs::write(
        host.join(".host-software"),
        format!("[software \"demo\"]\n\turl = {}\n\tpin = {pin}\n", src.to_string_lossy()),
    )
    .unwrap();
    let (code, _) = run(&["software", "--materialize", &host.to_string_lossy()]);
    assert_eq!(code, 0);
    let receipts = fs::read_to_string(host.join(RECEIPTS)).expect("the event was recorded");
    assert_eq!(receipts.matches("[receipt \"materialize\" \"demo\"]").count(), 1);
    assert!(host.join(ENVHASH).is_file(), "the state was recorded beside it");
    let _ = fs::remove_dir_all(&base);
}

// The advisory reader's exit split: nothing recorded is the one non-zero outcome,
// and it routes to the op that records one. A recorded tree exits zero.
#[test]
fn env_check_cannot_proceed_without_record() {
    let base = fixture("envcheck");
    fs::write(base.join(".host-software"), "").unwrap();
    let dir = base.to_string_lossy().to_string();
    let (code, text) = run(&["env", "--check", &dir]);
    assert_eq!(code, 2, "no fingerprint recorded yet");
    assert!(text.contains("--materialize"), "the message routes to the op that records one: {text}");

    let (src, pin) = seed_source(&base);
    fs::write(
        base.join(".host-software"),
        format!("[software \"demo\"]\n\turl = {}\n\tpin = {pin}\n", src.to_string_lossy()),
    )
    .unwrap();
    assert_eq!(run(&["software", "--materialize", &dir]).0, 0);
    let (code, text) = run(&["env", "--check", &dir]);
    assert_eq!(code, 0, "a recorded tree never gates: {text}");
    let _ = fs::remove_dir_all(&base);
}

// The gate's verdict: a tree missing a required artifact hazards and exits one,
// naming the remedy, and it writes neither of the two data files.
#[test]
fn verify_setup_hazarded_verdict() {
    let base = fixture("gate");
    fs::write(
        base.join(".host-software"),
        "[software \"ghost\"]\n\turl = u\n\tpin = 0000000000000000000000000000000000000000\n",
    )
    .unwrap();
    let dir = base.to_string_lossy().to_string();
    let (code, text) = run(&["software", "--verify-setup", &dir]);
    assert_eq!(code, 1, "a missing required artifact gates the setup");
    assert!(text.contains("--materialize"), "the hazard names the remedy: {text}");
    assert!(!base.join(ENVHASH).exists(), "the gate writes no fingerprint");
    assert!(!base.join(RECEIPTS).exists(), "the gate writes no provenance");
    let _ = fs::remove_dir_all(&base);
}

// The orchestrator ends in the gate and returns its verdict; a second run over the
// tree it made performs no step whose precondition now holds.
#[test]
fn bootstrap_completion_starts_the_gate() {
    let base = fixture("bootstrap");
    let (src, pin) = seed_source(&base);
    let host = base.join("host");
    fs::create_dir_all(&host).unwrap();
    fs::write(
        host.join(".host-software"),
        format!("[software \"demo\"]\n\turl = {}\n\tpin = {pin}\n", src.to_string_lossy()),
    )
    .unwrap();
    let dir = host.to_string_lossy().to_string();
    let (code, text) = run(&["bootstrap", &dir]);
    assert_eq!(code, 0, "the tree it made passes the gate it ends with: {text}");
    assert!(text.contains("verify the setup is complete"), "the gate is the last step: {text}");
    assert!(host.join("software").join("demo").join("main").is_dir(), "it materialized the tree");

    let (code2, text2) = run(&["bootstrap", &dir]);
    assert_eq!(code2, 0);
    assert!(text2.contains("skip     materialize"), "the second run skips what is done: {text2}");
    let _ = fs::remove_dir_all(&base);
}

// A step that FAILS ends the run: the orchestrator reports it and never reaches
// the gate, so nothing speaks for a setup it did not finish. (Distinct from a step
// it merely cannot perform, below.)
#[test]
fn bootstrap_abandons_on_a_failed_step() {
    let base = fixture("bootabandon");
    let (src, pin) = seed_source(&base);
    // The component offers a skill, so the link step has work to do.
    fs::create_dir_all(src.join("skills").join("tend")).unwrap();
    fs::write(src.join("skills").join("tend").join("SKILL.md"), "# tend\n").unwrap();
    git(&src, &["add", "-A"]);
    git(&src, &["commit", "-qm", "skill"]);
    let out = Command::new("git").arg("-C").arg(&src).args(["rev-parse", "HEAD"]).output().unwrap();
    let pin2 = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_ne!(pin, pin2);

    let host = base.join("host");
    fs::create_dir_all(&host).unwrap();
    // `.claude` is a FILE, so the link step cannot create its directory.
    fs::write(host.join(".claude"), "not a directory\n").unwrap();
    fs::write(
        host.join(".host-software"),
        format!("[software \"demo\"]\n\turl = {}\n\tpin = {pin2}\n", src.to_string_lossy()),
    )
    .unwrap();
    let (code, text) = run(&["bootstrap", &host.to_string_lossy()]);
    assert_eq!(code, 1, "the failed step ends the run: {text}");
    assert!(text.contains("skill"), "and says which step failed: {text}");
    assert!(!text.contains("setup complete"), "the gate never speaks for an unfinished run: {text}");
    assert!(!text.contains("install the commit hooks"), "later steps did not run: {text}");
    let _ = fs::remove_dir_all(&base);
}

// A step the orchestrator cannot perform does not end the run: the artifact it
// cannot build is reported as owed, the run reaches the gate anyway, and the gate
// states the verdict. Bootstrap never builds the recorded recipe itself — that
// recipe is written for the pinned toolchain container, not for whatever rust is
// on this machine.
#[test]
fn bootstrap_reaches_the_gate_after_an_unperformable_step() {
    let base = fixture("bootfail");
    let (src, pin) = seed_source(&base);
    // The component provides the commit gate but its recorded build cannot run, so
    // the step the hook install depends on fails.
    fs::write(src.join("hooks-script"), "#!/bin/bash\nexit 0\n").unwrap();
    git(&src, &["add", "-A"]);
    git(&src, &["commit", "-qm", "hooks"]);
    let out = Command::new("git").arg("-C").arg(&src).args(["rev-parse", "HEAD"]).output().unwrap();
    let pin2 = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_ne!(pin, pin2);
    let host = base.join("host");
    fs::create_dir_all(&host).unwrap();
    fs::write(
        host.join(".host-software"),
        format!(
            "[software \"gate\"]\n\turl = {}\n\tpin = {pin2}\n\thooks = hooks-script\n\tbuild = touch ambient-build-ran\n\tartifact = bin/gate 0000\n",
            src.to_string_lossy()
        ),
    )
    .unwrap();
    let dir = host.to_string_lossy().to_string();
    let (code, text) = run(&["bootstrap", &dir]);
    assert_eq!(code, 1, "the gate's verdict is the run's: {text}");
    assert!(text.contains("owed     gate artifact is absent"), "the owed artifact is named: {text}");
    assert!(text.contains("--verify-build"), "and the toolchain-correct way to produce it: {text}");
    assert!(
        !host.join("software").join("gate").join("main").join("ambient-build-ran").exists(),
        "the recorded build is never shelled into the ambient toolchain"
    );
    assert!(text.contains("HAZARD"), "the gate reports the gap it left: {text}");
    assert!(!text.contains("setup complete"), "and never reports a setup it did not finish");
    let _ = fs::remove_dir_all(&base);
}

/// A host fixture with the two rooms and a git repository, so the sweep's walk
/// (`git ls-files`) sees its documents.
fn host_fixture(name: &str) -> std::path::PathBuf {
    let base = fixture(name);
    git(&base, &["init", "-q", "-b", "main"]);
    git(&base, &["config", "user.email", "t@t"]);
    git(&base, &["config", "user.name", "t"]);
    fs::create_dir_all(base.join("plan").join("0074-materialize")).unwrap();
    fs::write(base.join("plan").join("0074-materialize").join("README.md"), "# m\n").unwrap();
    fs::create_dir_all(base.join("call")).unwrap();
    fs::write(base.join("call").join("0045-store-model.md"), "# d\n").unwrap();
    base
}

fn commit_all(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", "docs"]);
}

// The sweep's exit split, end to end: a tree whose references all resolve and
// render is clean; bare issue numbers advise; a dead register pointer gates.
#[test]
fn refs_check_splits_clean_advisory_and_dead() {
    let base = host_fixture("refs-split");
    let dir = base.to_string_lossy().to_string();

    fs::write(base.join("README.md"), "governed by [plan/0074](plan/0074-materialize/README.md)\n").unwrap();
    commit_all(&base);
    let (code, text) = run(&["refs", "--check", &dir]);
    assert_eq!(code, 0, "every reference resolves and renders: {text}");

    fs::write(base.join("README.md"), "see #17 for the reason\n").unwrap();
    commit_all(&base);
    let (code, text) = run(&["refs", "--check", &dir]);
    assert_eq!(code, 3, "a bare issue number advises: {text}");
    assert!(text.contains("Advisory"), "and says so: {text}");
    assert!(text.contains("--markdown"), "and names the remedy: {text}");

    fs::write(base.join("README.md"), "see plan/0099 and #17\n").unwrap();
    commit_all(&base);
    let (code, text) = run(&["refs", "--check", &dir]);
    assert_eq!(code, 1, "a dead pointer gates: {text}");
    assert!(text.contains("DEAD") && text.contains("plan/0099"), "naming it: {text}");
    let _ = fs::remove_dir_all(&base);
}

// The record layer is never reported. An append-only log is not rewritten to
// satisfy a checker, so the exclusion list the prose gate honours is the same one
// the sweep reads.
#[test]
fn refs_check_never_reports_the_record_layer() {
    let base = host_fixture("refs-record");
    let dir = base.to_string_lossy().to_string();
    fs::write(base.join("MEMORY.md"), "the append-only log cites #17 and #18 and plan/0099\n").unwrap();
    fs::write(base.join(".host-lintignore"), "MEMORY.md\n").unwrap();
    commit_all(&base);
    let (code, text) = run(&["refs", "--check", &dir]);
    assert_eq!(code, 0, "an excluded record is not swept, dead pointer or not: {text}");
    assert!(!text.contains("MEMORY.md"), "and is not named: {text}");
    let _ = fs::remove_dir_all(&base);
}

// Resolving one reference, through the real binary: the emission the caller asked
// for, and a usage exit for text that is not a reference.
#[test]
fn resolve_emits_the_form_asked_for() {
    let base = host_fixture("refs-resolve");
    let dir = base.to_string_lossy().to_string();
    let (code, text) = run(&["resolve", "plan/0074#write-spec", &dir]);
    assert_eq!(code, 0);
    assert!(text.trim().ends_with("README.md#write-spec"), "the anchor survives: {text}");
    let (code, text) = run(&["resolve", "call/0045", "--markdown", &dir]);
    assert_eq!(code, 0);
    assert!(text.contains("[call/0045](call/0045-store-model.md)"), "{text}");
    let (code, _) = run(&["resolve", "plan/74", &dir]);
    assert_eq!(code, 2, "text that is not a reference is a usage error");
    let (code, text) = run(&["resolve", "plan/0099", &dir]);
    assert_eq!(code, 1, "a reference this room cannot resolve fails: {text}");
    assert!(text.contains("unresolved here"), "{text}");
    let _ = fs::remove_dir_all(&base);
}

// The forge emissions, which no test reached until the review mutated them and
// watched every test stay green: a reference that names its repository resolves
// to that repository's tracker, and a bare number is refused rather than pointed
// at whichever remote happens to be local.
#[test]
fn resolve_builds_forge_urls_from_the_named_repository() {
    let base = host_fixture("refs-forge");
    git(&base, &["remote", "add", "origin", "https://github.com/anowner/arepo.git"]);
    fs::write(base.join("README.md"), "x\n").unwrap();
    commit_all(&base);
    let dir = base.to_string_lossy().to_string();

    let (code, text) = run(&["resolve", "anowner/other#17", "--url", &dir]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(text.trim(), "https://github.com/anowner/other/issues/17");

    let (code, text) = run(&["resolve", "other#17", "--markdown", &dir]);
    assert_eq!(code, 0, "a bare component name takes the origin's owner: {text}");
    assert_eq!(text.trim(), "[anowner/other#17](https://github.com/anowner/other/issues/17)");

    let (code, text) = run(&["resolve", "#17", "--url", &dir]);
    assert_eq!(code, 1, "a bare number names no repository: {text}");
    assert!(text.contains("names no repository"), "{text}");

    // A register URL uses the repository's own default branch, not a literal.
    let (code, text) = run(&["resolve", "plan/0074", "--url", &dir]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("/blob/main/plan/0074-materialize/README.md"), "{text}");
    let _ = fs::remove_dir_all(&base);
}

// A checker that cannot read anything must not report a clean tree, and the
// record layer is excluded whether or not the project has authored its list.
#[test]
fn refs_check_fails_closed_on_an_empty_corpus() {
    let base = fixture("refs-empty");
    let (code, text) = run(&["refs", "--check", &base.to_string_lossy()]);
    assert_eq!(code, 2, "nothing to sweep is not a pass: {text}");
    assert!(text.contains("no authored markdown"), "{text}");

    let host = host_fixture("refs-noignore");
    // No .host-lintignore at all, as a freshly scaffolded project has.
    fs::write(host.join("MEMORY.md"), "the log cites plan/0099 and #17\n").unwrap();
    fs::write(host.join("README.md"), "clean\n").unwrap();
    commit_all(&host);
    let (code, text) = run(&["refs", "--check", &host.to_string_lossy()]);
    assert_eq!(code, 0, "the append-only log is excluded by construction: {text}");
    assert!(!text.contains("MEMORY.md"), "{text}");
    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&host);
}

// The refusal a weak agent's invented flag deserves: the reason, and the action
// that does work.
#[test]
fn refs_fix_refuses_with_the_reason() {
    let base = host_fixture("refs-fix");
    fs::write(base.join("README.md"), "see #17\n").unwrap();
    commit_all(&base);
    let (code, text) = run(&["refs", "--fix", &base.to_string_lossy()]);
    assert_eq!(code, 2);
    assert!(text.contains("no --fix"), "{text}");
    assert!(text.contains("--markdown"), "and names what does work: {text}");
    assert!(text.contains("#17"), "naming a real reference from this tree, never a placeholder: {text}");
    let _ = fs::remove_dir_all(&base);
}
