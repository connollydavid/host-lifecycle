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

// A step that fails ends the run there: the orchestrator reports the failure rather
// than a setup it never finished, and the gate never speaks for it.
#[test]
fn bootstrap_abandons_on_step_failure() {
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
            "[software \"gate\"]\n\turl = {}\n\tpin = {pin2}\n\thooks = hooks-script\n\tbuild = exit 3\n\tartifact = bin/gate 0000\n",
            src.to_string_lossy()
        ),
    )
    .unwrap();
    let dir = host.to_string_lossy().to_string();
    let (code, text) = run(&["bootstrap", &dir]);
    assert_eq!(code, 1, "the failed step ends the run: {text}");
    assert!(text.contains("the commit gate needs its binary"), "it says which step failed: {text}");
    assert!(!text.contains("setup complete"), "and never reports a setup it did not finish");
    let _ = fs::remove_dir_all(&base);
}
