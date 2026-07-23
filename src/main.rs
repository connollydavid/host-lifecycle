//! host-lifecycle — the token-free lifecycle tool for an agentic project.
//!
//! Mechanical, rule-bound work — allocating zero-padded register numbers,
//! validating that names are well-formed, and scaffolding/stamping a repo when
//! it adopts the methodology — kept off the agent's token budget. Names come
//! from `host-grammar`, the same crate `host-lint` checks against, so what this
//! emits is exactly what the checker accepts.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use host_grammar::{format_number, is_valid_name, is_valid_slug};
use host_lint::{is_ci_file, is_scannable, path_ignored, scan_text_with_allow, Match, Severity};

mod mcp;
// plan/0073: the memory storage layer ships ahead of its consumers. The
// `dream` subcommand (#implement-dream) and the MCP `memory_*` tools
// (#extend-mcp) wire these surfaces; the allow lifts then.
#[allow(dead_code)]
mod memory;
// plan/0073 #implement-dream: the dream audit subcommand + detector engine.
// `dream` is wired below; the MCP memory_consolidate tool (#extend-mcp)
// reuses the engine.
mod dream;
mod bootstrap;
mod envhash;
mod setup;

/// The canonical template a project adopts from; recorded in the stamp.
const TEMPLATE_URL: &str = "https://github.com/connollydavid/host-template";
/// The migration stamp: records which template revision a repo adopted.
const STAMP: &str = ".host";
/// The lifecycle manifest (plan/0025): the single tool-readable journal of the
/// phase order + modality, in the template root, replacing the three prose copies.
/// One `[phase "<name>"]` stanza per phase, same git-config style as `.host-software`.
const MANIFEST: &str = "lifecycle.manifest";
/// The per-project receipts ledger (plan/0025): append-only, tool-written, one
/// stanza per recorded phase outcome. `software --check` re-verifies each by the
/// manifest's closed `recheck =` mechanism, never the receipt's own assertion.
const RECEIPTS: &str = ".host-receipts";
/// Operational lifecycle receipts (embed/release/verify/publish/classify/remap), split
/// out of `.host-receipts` (plan/0037). `.host-receipts` keeps the methodology-version
/// trail (adopt/upgrade) plus the applied-set; this file holds what host-lifecycle did.
const LIFECYCLE_RECEIPTS: &str = ".host-lifecycle-receipts";
/// The rooms `adopt` scaffolds (Where = the software submodule, added by hand).
const ROOMS: [&str; 3] = ["cast", "plan", "call"];

/// The verification-lane tools an adopter wires after `adopt` (the host pair plus
/// the requirements/timing lanes). `adopt` only scaffolds rooms and the stamp, so
/// it prints these as the remaining manual step — `(name, url)`, added under
/// `tools/<name>` and pinned to the commit the template references at this revision.
const TOOL_SUBMODULES: [(&str, &str); 4] = [
    ("host-lint", "https://github.com/connollydavid/host-lint"),
    ("host-lifecycle", "https://github.com/connollydavid/host-lifecycle"),
    ("allium", "https://github.com/juxt/allium"),
    ("specula", "https://github.com/specula-org/Specula"),
];

// The onboarding verbs (`init`/`adopt`, plan/0065) extend the exit-code convention with a
// machine-parseable contract for a scripted or agent caller, grounded in plan/0065 gather-data.md
// (a Fen acceptance run): 3 name-required (no name and no controlling terminal to prompt; a stderr
// line names the missing field so the caller re-invokes with --name), 4 target-exists (the
// `agentic-<name>` destination is present and non-empty; --force overrides), 5
// remote-failed-after-local-commit (the local project is intact and the manual remote command is
// printed). Usage stays 2, success 0.
const EXIT_NAME_REQUIRED: i32 = 3;
const EXIT_TARGET_EXISTS: i32 = 4;
const EXIT_REMOTE_FAILED: i32 = 5;

// Exit-code convention: 0 is clean or success; 1 is the red outcome a command exists to
// surface (a drift, a HAZARD, a failed gate); 2 is a command that cannot proceed on the input
// it was given (a usage error, or a missing, unreadable, or malformed input the user named: a
// directory, a file, the `.host-software`, the manifest). `next` returns 2 on a directory with
// no numbered entries (plan/0041), the same cannot-proceed class. The split (issues-found
// versus cannot-proceed) was validated with Qwen-3.5-4B in plan/0043 gather-data.

/// Map a program name (the arg0 basename) to the verb its shim invokes (plan/0065): `host-init`
/// runs `init`, `host-adopt` runs `adopt`. Any other name, `host-lifecycle` included, falls
/// through to the normal subcommand dispatch. The shims are the same binary under another name.
fn shim_verb(arg0_base: &str) -> Option<&'static str> {
    match arg0_base {
        "host-init" => Some("init"),
        "host-adopt" => Some("adopt"),
        _ => None,
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    // Shim dispatch (plan/0065): host-init and host-adopt are the same binary invoked under
    // another name (a symlink or copy the install places), giving a human a purpose-named
    // command over the engine verb. The program name maps to the verb; its args follow.
    let arg0_base = args
        .first()
        .and_then(|s| Path::new(s).file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if let Some(verb) = shim_verb(arg0_base) {
        match verb {
            "init" => init(&args[1..]),
            "adopt" => adopt(&args[1..]),
            _ => unreachable!("shim_verb returns only init/adopt"),
        }
        return;
    }
    match args.get(1).map(String::as_str) {
        Some("validate") => validate(args.get(2)),
        Some("next") => next(args.get(2)),
        Some("adopt") => adopt(&args[2..]),
        Some("init") => init(&args[2..]),
        Some("scaffold") => scaffold(&args[2..]),
        Some("mcp") => mcp::mcp(&args[2..]),
        Some("version") => version(args.get(2)),
        Some("classify") => classify(args.get(2)),
        Some("remap") => remap(&args[2..]),
        Some("software") => software(&args[2..]),
        Some("upgrade") => upgrade(&args[2..]),
        Some("book") => book(&args[2..]),
        Some("obligations") => obligations(&args[2..]),
        Some("manifest") => manifest(&args[2..]),
        Some("receipt") => receipt(&args[2..]),
        Some("release") => release(&args[2..]),
        Some("prose") => prose(&args[2..]),
        Some("reconcile") => reconcile(&args[2..]),
        Some("entrance") => entrance(&args[2..]),
        Some("migrate-receipts") => migrate_receipts(&args[2..]),
        Some("tasks") => tasks(&args[2..]),
        Some("dream") => dream::dream(&args[2..]),
        Some("env") => envhash::env(&args[2..]),
        Some("bootstrap") => bootstrap::bootstrap(&args[2..]),
        _ => {
            eprintln!("usage: host-lifecycle <validate|next|adopt|init|scaffold|mcp|version|classify|remap|software|upgrade|book|obligations|manifest|receipt|release|prose|reconcile|entrance|migrate-receipts|tasks> ...");
            eprintln!("  validate <dir>                — every NNNN-slug entry is well-formed");
            eprintln!("  next <dir>                    — print the next zero-padded number");
            eprintln!("  scaffold <dir> <rev> [--dry-run] — scaffold rooms + write the stamp (the primitive; call/0041)");
            eprintln!("  adopt [<source>] [--at <dir>] [--purpose <line>] [--name <name>] — three-route onboarding (refuse a software repo, adopt an empty agentic-<name> in place, else create the host elsewhere)");
            eprintln!("  init [<name>] [--at <dir>] [--purpose <line>] [--force] — create agentic-<name> as a fresh project (name backstop: flag/HOST_NAME/prompt, else exit 3)");
            eprintln!("  mcp — serve the onboarding tools (init/adopt) over an MCP stdio session, eliciting the name when the client supports it");
            eprintln!("  version <dir>                 — print the adopted template revision");
            eprintln!("  classify <dir>                — print the migration case (a|b|c); refuse a software repo");
            eprintln!("  remap --check <dir>           — tells left after the .host-remap dictionary applies");
            eprintln!("  remap --apply <dir> [--dry-run] — apply the dictionary (archive-first via a clean git tree)");
            eprintln!("  software --materialize <dir>  — clone the bare store(s) + worktrees from .host-software");
            eprintln!("  software --check <dir>        — verify each canonical worktree is at its recorded pin");
            eprintln!("  software --verify-build <dir> — rebuild from the pin and prove the artifact reproduces");
            eprintln!("  software --install-hooks <dir>— install each component's commit hooks + verified binary");
            eprintln!("  software --verify-setup <dir> — the completeness gate: every local artifact the recipe requires of this host is present");
            eprintln!("  software --teardown [--item <n>] <dir> — remove a component's worktrees + store (guards unsaved work; --force overrides)");
            eprintln!("  prose <dir>                   — audit authored markdown for prose tropes in-process (host-lint --docs; the verify recheck)");
            eprintln!("  reconcile <dir>               — re-check each `host-reconcile`-annotated restatement against the spine truth (the reflective-practice reconcile arm)");
            eprintln!("  entrance [--check] <dir>    — hold the single-file entrance to the spine: cover the phases + wired tools, generate the .host stamp block (plan/0040)");
            eprintln!("  migrate-receipts <dir>        — re-home the receipts family: applied-set to .host-receipts, operational receipts to .host-lifecycle-receipts (plan/0037)");
            eprintln!("  upgrade <dir>                 — list template UPGRADING.md actions newer than the stamp");
            eprintln!("  book <dir> [--dry-run]        — generate mdBook/src/ + SUMMARY.md (lifecycle order) for mdBook");
            eprintln!("  book --check <dir>            — fail unless every room renders at least one page");
            eprintln!("  obligations <spec.allium>     — every `allium plan` obligation is dispositioned in <stem>.obligations");
            eprintln!("  manifest --check <path>       — the lifecycle manifest is well-formed (orders unique, requires resolve)");
            eprintln!("  receipt --record <phase> ...  — append a phase receipt (done|skip); --list prints the current set");
            eprintln!("  release <component> ...       — the gated, tool-carried release sequence (verify -> build -> tag -> receipt)");
            eprintln!("  env --check <dir>             — which local-environment dimensions moved since the fingerprint was recorded (advisory)");
            eprintln!("  bootstrap <dir>               — run the fresh-clone setup sequence (submodules, materialize, skills, build, hooks, re-deriver), then the completeness gate");
            process::exit(2);
        }
    }
}

/// Entries in a register dir (`plan/`, `call/`, …) whose name starts with a
/// digit, with any trailing `.md` stripped so files and folders read alike.
fn numbered_entries(dir: &Path) -> Vec<String> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("host-lifecycle: cannot read {}: {e}", dir.display());
            process::exit(2);
        }
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .map(|n| n.strip_suffix(".md").map(str::to_string).unwrap_or(n))
        .filter(|n| n.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    names.sort();
    names
}

fn validate(dir: Option<&String>) {
    let Some(dir) = dir else {
        eprintln!("host-lifecycle validate <dir>");
        process::exit(2);
    };
    let path = Path::new(dir);
    let mut bad = 0;
    for name in numbered_entries(path) {
        if !is_valid_name(&name) {
            println!("invalid: {name}");
            bad += 1;
        }
    }
    // The Why room is also scope-gated (anti-ouroboros); other rooms are name-only.
    if path.file_name().and_then(|s| s.to_str()) == Some("call") {
        bad += validate_call_scope(path);
    }
    if bad > 0 {
        eprintln!("{bad} problem(s)");
        process::exit(1);
    }
    println!("ok");
}

/// Scope-gate every numbered decision in a `call/` dir; returns the offender count.
fn validate_call_scope(dir: &Path) -> usize {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().and_then(|s| s.to_str()) == Some("md")
                && p.file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit()))
        })
        .collect();
    files.sort();
    let mut bad = 0;
    for p in files {
        let text = fs::read_to_string(&p).unwrap_or_default();
        if let Some(problem) = decision_scope_problem(&text) {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            println!("{name}: {problem}");
            bad += 1;
        }
    }
    bad
}

fn next(dir: Option<&String>) {
    let Some(dir) = dir else {
        eprintln!("usage: host-lifecycle next <dir>");
        process::exit(2);
    };
    let path = Path::new(dir);
    match next_number(path) {
        Ok(n) => println!("{}", format_number(n)),
        Err(NextError::NotDir) => {
            eprintln!("host-lifecycle: `{dir}` is not a directory");
            eprintln!("  point `next` at a room such as plan/ or call/");
            process::exit(2);
        }
        Err(NextError::Empty) => {
            eprintln!("host-lifecycle: `{dir}` has no numbered (NNNN-slug) entries");
            match rooms_with_entries(path).first() {
                Some(room) => eprintln!("  did you mean a room? try: host-lifecycle next {room}"),
                None => eprintln!("  point `next` at a room such as plan/ or call/ (a room's first entry is 0000)"),
            }
            process::exit(2);
        }
    }
}

/// `next` fails closed (plan/0041): a path that is not a directory, or a directory
/// with no `NNNN-slug` entries, has no well-defined next number. The retired `0000`
/// fallback returned a plausible wrong answer for a typo'd or non-room path.
enum NextError {
    NotDir,
    Empty,
}

fn next_number(dir: &Path) -> Result<u32, NextError> {
    if !dir.is_dir() {
        return Err(NextError::NotDir);
    }
    numbered_entries(dir)
        .iter()
        .filter_map(|n| n.split('-').next())
        .filter_map(|num| num.parse::<u32>().ok())
        .max()
        .map(|m| m + 1)
        .ok_or(NextError::Empty)
}

/// The methodology rooms under `dir` that already hold a numbered entry, in room
/// order — the real rooms `next` suggests when it is pointed at a parent (a repo
/// root, a typo) that has none of its own. Restricted to the known rooms so a
/// generated or build directory is never suggested.
fn rooms_with_entries(dir: &Path) -> Vec<String> {
    ROOMS
        .iter()
        .filter(|room| dir_has_numbered_entry(&dir.join(room)))
        .map(|room| (*room).to_string())
        .collect()
}

fn dir_has_numbered_entry(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|rd| {
        rd.filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with(|c: char| c.is_ascii_digit()))
    })
}

/// `scaffold <dir> <revision> [--dry-run]` — scaffold the rooms a host needs and
/// write the `.host` stamp recording the template revision adopted (call/0041: the
/// primitive that `adopt` was renamed from). Idempotent: existing rooms are left
/// untouched. `--dry-run` writes nothing.
fn scaffold(args: &[String]) {
    let mut dry = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--dry-run" => dry = true,
            _ => pos.push(a),
        }
    }
    let (Some(dir), Some(revision)) = (pos.first(), pos.get(1)) else {
        eprintln!("host-lifecycle scaffold <dir> <revision> [--dry-run]");
        process::exit(2);
    };
    let root = Path::new(dir.as_str());
    if !root.is_dir() {
        eprintln!("host-lifecycle: not a directory: {}", root.display());
        process::exit(2);
    }

    scaffold_rooms_stamp(root, revision, dry);
    print_scaffold_checklist(revision);
}

/// Scaffold the rooms, the `.host` stamp, and the LEXICON scaffold into `root`, shared by
/// `adopt` (into an existing repo) and `init` (into a fresh `agentic-<name>`). Prints one
/// line per artifact and honours `dry`.
fn scaffold_rooms_stamp(root: &Path, revision: &str, dry: bool) {
    for room in ROOMS {
        let p = root.join(room);
        if p.is_dir() {
            println!("skip   {room}/ (exists)");
        } else if dry {
            println!("create {room}/ (dry-run)");
        } else {
            if let Err(e) = fs::create_dir_all(&p) {
                eprintln!("host-lifecycle: cannot create {}: {e}", p.display());
                process::exit(2);
            }
            // Empty dirs do not survive git; leave a keepfile so the room ships.
            let _ = fs::write(p.join(".gitkeep"), b"");
            println!("create {room}/");
        }
    }

    let body = stamp_body(revision, &today());
    let stamp = root.join(STAMP);
    if dry {
        println!("write  {STAMP} (revision {revision}) (dry-run)");
    } else if let Err(e) = fs::write(&stamp, body) {
        eprintln!("host-lifecycle: cannot write {}: {e}", stamp.display());
        process::exit(2);
    } else {
        println!("write  {STAMP} (revision {revision})");
    }

    seed_lexicon(root, dry);
}

/// The starter `LEXICON` seeded at adoption: a comment-only scaffold documenting
/// the format and how to opt into strict / jira-key gating (issue #13). No entries
/// and no active directive, so it never blocks an existing repo — the operator
/// audits (`host-lint --all`) and curates with `host-lint lexicon add`. The example
/// tokens use all-caps version designators (`NT 3.1`), which host-lint recognises as
/// version strings, so the scaffold itself never trips the linter.
const LEXICON_SEED: &str = "\
# LEXICON — sanctioned tell-shaped tokens for host-lint (see the host-lint README).
# One entry per line is the full contextual phrase that legitimizes a token
# (a version string like NT 3.1, a product like COM1), masked before detection;
# a tracker reference carries its URL on the same line (#7 then the link).
#
# Do NOT hand-author entries — the tool owns every decision:
#   host-lint lexicon add \"<phrase>\" [--url <url>]
#
# After auditing the repo (host-lint --all) and curating the legitimate tokens,
# opt into escalation by adding one of these directive lines (drop the leading #):
#   host-lint: strict            an undeclared tell-shaped token blocks, not warns
#   host-lint: jira-key PROJ     gate a tracker key: PROJ-NNNN entries need a URL
";

/// Seed the LEXICON scaffold at the host root, skipping if one already exists.
fn seed_lexicon(root: &Path, dry: bool) {
    let p = root.join("LEXICON");
    if p.exists() {
        println!("skip   LEXICON (exists)");
    } else if dry {
        println!("write  LEXICON (scaffold) (dry-run)");
    } else if let Err(e) = fs::write(&p, LEXICON_SEED) {
        eprintln!("host-lifecycle: cannot write {}: {e}", p.display());
        process::exit(2);
    } else {
        println!("write  LEXICON (scaffold)");
    }
}

/// Print the post-`scaffold` checklist. `scaffold` writes rooms and the stamp only;
/// registering the template + verification tools and installing the hooks is manual
/// work with no other prompt, so spell it out. The template submodule is step 1: the
/// `upgrade` phase reads `UPGRADING.md` from it, so an adoption that skips it makes
/// the very next phase fail with no ledger to read.
fn print_scaffold_checklist(revision: &str) {
    println!("\nnext steps (scaffold writes rooms + the stamp only):");
    println!("  1. register the methodology template as a submodule at the adopted");
    println!("     revision — `upgrade` reads its `UPGRADING.md` ledger:");
    println!("       git submodule add {TEMPLATE_URL} host-template");
    println!("       (cd host-template && git checkout {revision})");
    println!("  2. wire the verification tools as submodules, each pinned to the commit");
    println!("     the template references at this revision:");
    for (name, url) in TOOL_SUBMODULES {
        println!("       git submodule add {url} tools/{name}");
    }
    println!("  3. embed the software in the Where slot (.host-software), record a");
    println!("     `hooks` and `artifact` for the gating tool, and run:");
    println!("       host-lifecycle software --materialize .");
    println!("  4. build the gating tool, then install its commit hooks + binary:");
    println!("       host-lifecycle software --install-hooks .");
}

/// Resolve the project name from an explicit value or the `HOST_NAME` environment variable
/// (plan/0065 backstop, the pure half). Returns `None` when neither supplies a non-empty
/// name, so the caller falls back to a terminal prompt or the name-required exit.
fn resolve_name_from(explicit: Option<&str>, env_name: Option<&str>) -> Option<String> {
    [explicit, env_name]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|t| !t.is_empty())
        .map(str::to_string)
}

/// Resolve the project name per the full backstop contract (plan/0065): an explicit value
/// or `HOST_NAME` wins; else prompt on a controlling terminal; else exit 3 (name-required)
/// with a machine-parseable stderr line, so a scripted or agent caller re-invokes with --name.
fn resolve_name(explicit: Option<&str>) -> String {
    let env_name = env::var("HOST_NAME").ok();
    if let Some(n) = resolve_name_from(explicit, env_name.as_deref()) {
        return n;
    }
    use std::io::{IsTerminal as _, Write as _};
    if std::io::stdin().is_terminal() {
        eprint!("project name: ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() {
            if let Some(n) = resolve_name_from(Some(&line), None) {
                return n;
            }
        }
    }
    eprintln!("name-required: supply the project name with --name <name> or the HOST_NAME environment variable");
    process::exit(EXIT_NAME_REQUIRED);
}

/// Validate the project name as a host-grammar slug (the naming authority): the same slug
/// shape `host-lint` accepts, so what `init` coins is what the checker allows.
fn validate_slug(name: &str) -> Result<(), String> {
    if is_valid_slug(name) {
        Ok(())
    } else {
        Err(format!(
            "`{name}` is not a valid slug (lowercase a-z, 0-9, single internal hyphens; no leading, trailing, or doubled hyphen)"
        ))
    }
}

/// The MEMORY.md a fresh `init` writes, seeding the one-line purpose (plan/0065 ruled seed).
/// A `None` or empty purpose leaves a default MEMORY.md with no purpose line.
fn memory_seed_body(purpose: Option<&str>) -> String {
    let mut body = String::from("# MEMORY.md\n\n## Session Log\n");
    if let Some(p) = purpose {
        let p = p.trim();
        if !p.is_empty() {
            body.push_str(&format!("\n- Project purpose: {p}\n"));
        }
    }
    body
}

/// The machine-readable handoff block (plan/0065 line-based contract): the created path,
/// the remote if any, and the next command. The same output serves a scripted or agent caller.
fn handoff_block(host_path: &str, remote: Option<&str>) -> String {
    let mut s = format!("host-path: {host_path}\n");
    if let Some(r) = remote {
        s.push_str(&format!("remote: {r}\n"));
    }
    s.push_str(&format!("next: cd {host_path}\n"));
    s
}

/// Resolve the latest host-template revision to stamp a fresh project, via `git ls-remote`.
/// Onboarding is an online operation (it also wires the remote), so an unreachable template
/// is a clear error naming the `--revision` override rather than a silent default.
fn resolve_template_revision() -> String {
    match process::Command::new("git")
        .args(["ls-remote", TEMPLATE_URL, "HEAD"])
        .output()
    {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            match text.split_whitespace().next() {
                Some(sha) if sha.len() >= 7 => sha.to_string(),
                _ => {
                    eprintln!("host-lifecycle init: could not parse the template revision; pass --revision <rev>");
                    process::exit(2);
                }
            }
        }
        _ => {
            eprintln!("host-lifecycle init: could not reach the template ({TEMPLATE_URL}); pass --revision <rev>");
            process::exit(2);
        }
    }
}

/// `init [<name>] [--at <dir>] [--purpose <line>] [--revision <rev>] [--force]` — the
/// fresh-folder onboarding path (plan/0065, the `cargo new` shape). Create `agentic-<name>`
/// as a new directory under `--at` (default the working directory), scaffold its rooms and
/// stamp, seed the one-line purpose into MEMORY.md, and print the machine-readable handoff.
/// The name follows the backstop contract; a present, non-empty target refuses (exit 4).
/// Create a fresh `agentic-<name>` project under `parent`: scaffold its rooms and stamp, a
/// README heading, and MEMORY.md seeded with the one-line purpose. Returns the target path.
/// Refuses a present, non-empty target (exit 4) unless `force`. Shared by `init` (a fresh
/// folder) and `adopt`'s create-elsewhere route (plan/0065).
fn create_project(name: &str, parent: &Path, purpose: Option<&str>, revision: &str, force: bool) -> PathBuf {
    let target = parent.join(format!("agentic-{name}"));
    if target.exists() {
        let empty = fs::read_dir(&target).map(|mut d| d.next().is_none()).unwrap_or(false);
        if !(force || empty) {
            eprintln!("target-exists: {} already exists and is not empty (use --force)", target.display());
            process::exit(EXIT_TARGET_EXISTS);
        }
    }
    if let Err(e) = fs::create_dir_all(&target) {
        eprintln!("host-lifecycle: cannot create {}: {e}", target.display());
        process::exit(2);
    }
    scaffold_rooms_stamp(&target, revision, false);
    // A README heading so the project has an entrance; the one-line purpose lives in MEMORY.
    let readme = target.join("README.md");
    if !readme.exists() {
        let _ = fs::write(&readme, format!("# agentic-{name}\n"));
        println!("write  README.md");
    }
    seed_memory(&target, purpose);
    target
}

/// Write MEMORY.md seeded with the one-line purpose (plan/0065 ruled seed), leaving an
/// existing MEMORY untouched so an in-place adopt never clobbers a project's memory.
fn seed_memory(root: &Path, purpose: Option<&str>) {
    let mem = root.join("MEMORY.md");
    if mem.exists() {
        println!("skip   MEMORY.md (exists)");
        return;
    }
    if let Err(e) = fs::write(&mem, memory_seed_body(purpose)) {
        eprintln!("host-lifecycle: cannot write {}: {e}", mem.display());
        process::exit(2);
    }
    let seeded = purpose.map(|p| !p.trim().is_empty()).unwrap_or(false);
    println!("write  MEMORY.md{}", if seeded { " (purpose)" } else { "" });
}

/// The `<name>` of a folder named `agentic-<name>` with a valid slug, else `None`. Route 2
/// of onboarding (plan/0065) takes the name from such a folder rather than prompting.
fn agentic_basename(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.strip_prefix("agentic-")?;
    is_valid_slug(name).then(|| name.to_string())
}

/// Initialise a git repo in `dir` and commit the scaffold (plan/0065 oneshot: commit before
/// wiring the remote). Prefers the user's git identity; sets a local fallback only when none is
/// configured, so a fresh machine still commits. A clean "nothing to commit" is not a failure.
fn git_init_commit(dir: &Path, message: &str) -> Result<(), String> {
    let git = |args: &[&str]| -> Result<std::process::Output, String> {
        process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .map_err(|e| format!("git: {e}"))
    };
    if !dir.join(".git").exists() {
        let o = git(&["init", "-q"])?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
    }
    let has_id = git(&["config", "user.email"])
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);
    if !has_id {
        let _ = git(&["config", "user.email", "host-lifecycle@localhost"]);
        let _ = git(&["config", "user.name", "host-lifecycle"]);
    }
    // Stage only the scaffold artifacts (not a blanket `-A`), so a `--force` reuse of a populated
    // repo never sweeps unrelated working-tree files into the scaffold commit.
    const SCAFFOLD: [&str; 7] = ["cast", "plan", "call", ".host", "LEXICON", "README.md", "MEMORY.md"];
    let present: Vec<&str> = SCAFFOLD.iter().copied().filter(|p| dir.join(p).exists()).collect();
    if present.is_empty() {
        return Ok(()); // nothing scaffolded to commit
    }
    let mut add_args: Vec<&str> = vec!["add", "--"];
    add_args.extend_from_slice(&present);
    let o = git(&add_args)?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    let o = git(&["commit", "-q", "-m", message])?;
    if !o.status.success() {
        // A "nothing to commit" outcome (git prints it to stdout) is a clean no-op, not a failure.
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        if !combined.contains("nothing to commit") {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
    }
    Ok(())
}

/// Whether the GitHub CLI is present and authenticated. A false here degrades onboarding to a
/// committed-local-only success rather than an error (the remote is optional and fully flagged).
fn gh_available() -> bool {
    process::Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create the `agentic-<name>` GitHub repository from `dir` and push, returning the remote URL.
/// Private unless `public`. Errors carry gh's stderr for the caller's remote-failed handling.
fn github_create_push(dir: &Path, name: &str, public: bool) -> Result<String, String> {
    let vis = if public { "--public" } else { "--private" };
    let o = process::Command::new("gh")
        .arg("repo")
        .arg("create")
        .arg(format!("agentic-{name}"))
        .arg(vis)
        .arg("--source")
        .arg(dir)
        .arg("--push")
        .output()
        .map_err(|e| format!("gh: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    let remote = process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("agentic-{name}"));
    Ok(remote)
}

/// Wire the new project's git repo and optional remote (plan/0065 oneshot: scaffold, commit,
/// then create-and-push the remote last). Commits the scaffold unless `no_git`. With `github`,
/// creates the repo (private unless `public`) and pushes; a remote failure after the local commit
/// leaves the project intact, prints the manual remote steps, and exits 5. Returns the remote URL
/// when one was created.
fn finalize_onboarding(target: &Path, name: &str, no_git: bool, github: bool, public: bool) -> Option<String> {
    // The --no-git + --github conflict is caught at the verb's preflight, before any scaffold.
    if no_git {
        return None;
    }
    if let Err(e) = git_init_commit(target, &format!("scaffold agentic-{name}")) {
        eprintln!("host-lifecycle: could not initialise the local git repo: {e}");
        process::exit(2);
    }
    println!("git    initialised and committed the scaffold");
    if !github {
        return None;
    }
    if !gh_available() {
        println!("note   gh is absent or unauthenticated; the project is committed locally only");
        return None;
    }
    match github_create_push(target, name, public) {
        Ok(url) => {
            println!("github created and pushed {url}");
            Some(url)
        }
        Err(e) => {
            // remote-failed-after-local-commit: the local project is intact (plan/0065 exit 5).
            eprintln!("host-lifecycle: the remote step failed; the local project is intact: {e}");
            eprintln!("  create and push the remote by hand, from {}:", target.display());
            eprintln!(
                "    gh repo create agentic-{name} --{} --source . --push",
                if public { "public" } else { "private" }
            );
            process::exit(EXIT_REMOTE_FAILED);
        }
    }
}

/// `init [<name>] [--at <dir>] [--purpose <line>] [--revision <rev>] [--force]` — the
/// fresh-folder onboarding path (plan/0065, the `cargo new` shape). Create `agentic-<name>`
/// as a new directory under `--at` (default the working directory), scaffold its rooms and
/// stamp, seed the one-line purpose into MEMORY.md, and print the machine-readable handoff.
/// The name follows the backstop contract; a present, non-empty target refuses (exit 4).
fn init(args: &[String]) {
    let mut explicit_name: Option<String> = None;
    let mut at: Option<String> = None;
    let mut purpose: Option<String> = None;
    let mut revision: Option<String> = None;
    let mut force = false;
    let mut github = false;
    let mut public = false;
    let mut no_git = false;
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => { i += 1; explicit_name = args.get(i).cloned(); }
            "--at" => { i += 1; at = args.get(i).cloned(); }
            "--purpose" => { i += 1; purpose = args.get(i).cloned(); }
            "--revision" => { i += 1; revision = args.get(i).cloned(); }
            "--force" => force = true,
            "--github" => github = true,
            "--public" => public = true,
            "--no-git" => no_git = true,
            other if other.starts_with("--") => {
                eprintln!("host-lifecycle init: unknown flag {other}");
                process::exit(2);
            }
            other => pos.push(other.to_string()),
        }
        i += 1;
    }
    if pos.len() > 1 {
        eprintln!("host-lifecycle init [<name>] [--at <dir>] [--purpose <line>] [--revision <rev>] [--force] [--github [--public]] [--no-git]");
        process::exit(2);
    }
    // Preflight the conflicting-flags case before any write (plan/0065: preflight before scaffold).
    if no_git && github {
        eprintln!("host-lifecycle init: --github needs a commit; drop --no-git");
        process::exit(2);
    }
    let name_arg = explicit_name.or_else(|| pos.into_iter().next());
    let name = resolve_name(name_arg.as_deref());
    if let Err(msg) = validate_slug(&name) {
        eprintln!("host-lifecycle init: {msg}");
        process::exit(2);
    }
    let parent = PathBuf::from(at.as_deref().unwrap_or("."));
    let rev = revision.unwrap_or_else(resolve_template_revision);
    let target = create_project(&name, &parent, purpose.as_deref(), &rev, force);
    let remote = finalize_onboarding(&target, &name, no_git, github, public);
    print!("\n{}", handoff_block(&target.display().to_string(), remote.as_deref()));
}

/// `adopt [<source>] [--at <dir>] [--purpose <line>] [--name <name>] [--revision <rev>] [--force]`
/// — the three-route onboarding (plan/0065, call/0061), invoked by a human through the
/// `host-adopt` shim. Route by the source folder (default the working directory): a software
/// repository is refused in place with embed-elsewhere steps (route one); an empty
/// `agentic-<name>` folder is adopted in place, its name taken from the folder (route two); any
/// other folder is arbitrary, so elicit a name, create the host at `../agentic-<name>` (override
/// `--at`), and leave the source untouched (route three). The deprecated `adopt <dir> <revision>`
/// primitive form forwards to `scaffold` with a deprecation notice (call/0041), retiring later.
fn adopt(args: &[String]) {
    let mut at: Option<String> = None;
    let mut purpose: Option<String> = None;
    let mut explicit_name: Option<String> = None;
    let mut revision: Option<String> = None;
    let mut force = false;
    let mut github = false;
    let mut public = false;
    let mut no_git = false;
    let mut pos: Vec<String> = Vec::new();
    let mut end_of_opts = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if end_of_opts {
            pos.push(a.to_string());
            i += 1;
            continue;
        }
        match a {
            // an explicit end-of-options, so a source path that begins with `-` is not read as a flag
            "--" => end_of_opts = true,
            "--at" => { i += 1; at = args.get(i).cloned(); }
            "--purpose" => { i += 1; purpose = args.get(i).cloned(); }
            "--name" => { i += 1; explicit_name = args.get(i).cloned(); }
            "--revision" => { i += 1; revision = args.get(i).cloned(); }
            "--force" => force = true,
            "--github" => github = true,
            "--public" => public = true,
            "--no-git" => no_git = true,
            other if other.starts_with("--") => {
                eprintln!("host-lifecycle adopt: unknown flag {other}");
                process::exit(2);
            }
            other => pos.push(other.to_string()),
        }
        i += 1;
    }

    // `adopt` takes at most one positional (the source folder). Two or more is a hard error, never a
    // silent forward: the onboarding grammar cannot safely disambiguate the retired
    // `adopt <dir> <revision>` primitive from a mistaken `adopt <source> <name>`, and forwarding the
    // latter to `scaffold` would write into the source, breaking the source-read-only invariant. The
    // primitive lives on under its own name (call/0041; the review found the forward unsafe).
    if pos.len() > 1 {
        eprintln!("host-lifecycle adopt takes at most one positional <source>.");
        eprintln!("  for the renamed scaffold+stamp primitive, use: host-lifecycle scaffold <dir> <revision>");
        eprintln!("  to name the new host, use --name: host-lifecycle adopt [<source>] --name <name>");
        process::exit(2);
    }
    if no_git && github {
        eprintln!("host-lifecycle adopt: --github needs a commit; drop --no-git");
        process::exit(2);
    }

    let source_raw = Path::new(pos.first().map(String::as_str).unwrap_or("."));
    if !source_raw.is_dir() {
        eprintln!("host-lifecycle: not a directory: {}", source_raw.display());
        process::exit(2);
    }
    let source = source_raw.canonicalize().unwrap_or_else(|_| source_raw.to_path_buf());

    match adopt_route(&source, force) {
        // Route one: a software repository is refused in place, with the embed-elsewhere steps. The
        // refuse outcome is exit 4 (the ruled contract's "target-exists or refuse"), distinct from a
        // usage error (2), so a scripted caller can tell a principled refusal from a bad invocation.
        AdoptRoute::Refuse(manifest) => {
            eprint!("{}", refuse_adopt_in_place(&source.display().to_string(), manifest));
            process::exit(EXIT_TARGET_EXISTS);
        }
        // Route two: an effectively-empty `agentic-<name>` folder is adopted in place, its name from
        // the folder. The oneshot finalize (git commit, optional remote) applies here too.
        AdoptRoute::InPlace(name) => {
            let rev = revision.unwrap_or_else(resolve_template_revision);
            println!("adopt in place: {} (name {name}, taken from the folder)", source.display());
            scaffold_rooms_stamp(&source, &rev, false);
            seed_memory(&source, purpose.as_deref());
            print_scaffold_checklist(&rev);
            let remote = finalize_onboarding(&source, &name, no_git, github, public);
            print!("\n{}", handoff_block(&source.display().to_string(), remote.as_deref()));
        }
        // Route three: an arbitrary folder — elicit a name, create the host elsewhere, source untouched.
        AdoptRoute::Elsewhere => {
            let name = resolve_name(explicit_name.as_deref());
            if let Err(msg) = validate_slug(&name) {
                eprintln!("host-lifecycle adopt: {msg}");
                process::exit(2);
            }
            let parent = match at.as_deref() {
                Some(a) => PathBuf::from(a),
                None => source.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("..")),
            };
            // The source is read-only on route three, so the new host must land outside it. Resolve
            // the parent (best effort for a not-yet-existing --at) and refuse a target within source.
            let parent_abs = parent
                .canonicalize()
                .or_else(|_| env::current_dir().map(|c| c.join(&parent)))
                .unwrap_or_else(|_| parent.clone());
            let target_preview = parent_abs.join(format!("agentic-{name}"));
            if target_preview == source || target_preview.starts_with(&source) {
                eprintln!("host-lifecycle adopt: the new host would land inside the source {}; route three keeps the source untouched, so choose an --at outside it", source.display());
                process::exit(2);
            }
            let rev = revision.unwrap_or_else(resolve_template_revision);
            let target = create_project(&name, &parent, purpose.as_deref(), &rev, force);
            let remote = finalize_onboarding(&target, &name, no_git, github, public);
            println!("\nadopted from {}: the source folder is untouched. Switch to the new host:", source.display());
            print!("{}", handoff_block(&target.display().to_string(), remote.as_deref()));
        }
    }
}

/// The onboarding route for a source folder (plan/0065): a software repository is refused
/// (route one, carrying the manifest reason); an unstamped, empty `agentic-<name>` folder is
/// adopted in place (route two, carrying the name); anything else creates a host elsewhere
/// (route three). `--force` lets route two claim a non-empty `agentic-<name>`. Pure, so the
/// routing decision is unit-tested without the side effects the verb wraps it in.
enum AdoptRoute {
    Refuse(&'static str),
    InPlace(String),
    Elsewhere,
}

fn adopt_route(source: &Path, force: bool) -> AdoptRoute {
    if let Some(manifest) = adopt_in_place_refusal(source) {
        return AdoptRoute::Refuse(manifest);
    }
    if let Some(name) = agentic_basename(source) {
        let unstamped_empty = !source.join(STAMP).is_file() && dir_effectively_empty(source);
        if unstamped_empty || force {
            return AdoptRoute::InPlace(name);
        }
    }
    AdoptRoute::Elsewhere
}

/// Whether a folder is empty enough to adopt in place (plan/0065 route two): it carries no entries
/// beyond the artifacts a freshly created or cloned repo brings (`.git`, `.gitignore`, a README, a
/// LICENSE), so a `git init agentic-<name>` or a fresh `gh repo create --clone` still routes in place.
fn dir_effectively_empty(dir: &Path) -> bool {
    const CARRIED: [&str; 5] = [".git", ".gitignore", "README.md", "LICENSE", "LICENSE.md"];
    match fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(Result::ok).all(|e| {
            e.file_name()
                .to_str()
                .map(|n| CARRIED.contains(&n))
                .unwrap_or(false)
        }),
        Err(_) => false,
    }
}

/// `version <dir>` — print the template revision recorded in the stamp.
fn version(dir: Option<&String>) {
    let Some(dir) = dir else {
        eprintln!("host-lifecycle version <dir>");
        process::exit(2);
    };
    let Ok(stamp) = fs::read_to_string(Path::new(dir).join(STAMP)) else {
        eprintln!("host-lifecycle: no readable {STAMP} in {dir}");
        process::exit(1);
    };
    // The applied state, not just the legacy revision: a single `revision` would
    // hide an `applied` set and mislead about what is actually adopted (plan/0022).
    if let Some(b) = baseline_field(&stamp) {
        println!("baseline {b}");
        let applied = read_applied_ids(Path::new(dir));
        if !applied.is_empty() {
            println!("applied {}: {}", applied.len(), applied.join(" "));
        }
    } else if let Some(rev) = parse_revision(&stamp) {
        println!("revision {rev} (legacy stamp — run `host-lifecycle upgrade` to migrate to a baseline)");
    } else {
        eprintln!("host-lifecycle: {STAMP} in {dir} has neither baseline nor revision");
        process::exit(1);
    }
}

/// `classify <dir>` — print the migration case: `c` if the repo carries a stamp
/// (ours, prior), `b` if it has a CLAUDE.md but no stamp (foreign governance),
/// `a` if it has neither (greenfield).
fn classify(dir: Option<&String>) {
    let Some(dir) = dir else {
        eprintln!("host-lifecycle classify <dir>");
        process::exit(2);
    };
    let root = Path::new(dir);
    if let Some(manifest) = adopt_in_place_refusal(root) {
        eprint!("{}", refuse_adopt_in_place(dir, manifest));
        process::exit(3);
    }
    println!(
        "{}",
        classify_case(root.join(STAMP).is_file(), root.join("CLAUDE.md").is_file())
    );
}

/// The refusal message: why adopting software in place is forbidden, and the
/// exact steps to embed it as the Where room of a separate host instead.
fn refuse_adopt_in_place(dir: &str, manifest: &str) -> String {
    format!(
        "refuse: {dir} is a software repository ({manifest} at its root), not an empty \
or agentic-host folder.\n\n\
This methodology never turns a software repository into a host. Software is\n\
embedded into a *host* (a separate meta-repo) as the Where room — a bare store\n\
with worktrees recorded in .host-software — so the code and the governance stay\n\
separable and independently versioned.\n\n\
To proceed:\n\
\x20 1. Create or choose a host repository, separate from this software\n\
\x20    (e.g. a new empty repo `agentic-<name>`).\n\
\x20 2. In the host, run: host-lifecycle scaffold <host-dir> <revision>\n\
\x20    (scaffolds the rooms and writes the .host stamp).\n\
\x20 3. Embed THIS software as the Where room: add a [software \"<name>\"] stanza to\n\
\x20    the host's .host-software (this repo's URL, a pinned SHA, the worktree set),\n\
\x20    gitignore the trees, then: host-lifecycle software --materialize <host-dir>.\n\
\x20 4. Wire the tools and verify (host README, steps 3 and 6).\n\n\
Do not run adopt inside this software repository.\n"
    )
}

/// The stamp file body — a plain key/value record of the adopted template.
fn stamp_body(revision: &str, date: &str) -> String {
    format!("template = \"{TEMPLATE_URL}\"\nrevision = \"{revision}\"\nadopted  = \"{date}\"\n")
}

/// Pull the `revision` value out of a stamp file's text.
fn parse_revision(text: &str) -> Option<String> {
    stamp_field(text, "revision")
}

/// The value after `=` in a stamp line: the contents of the first double-quoted
/// run if present, else the first whitespace-delimited token. An inline
/// `# comment` outside quotes is ignored. Empty counts as absent.
fn stamp_value_after_eq(rest: &str) -> Option<String> {
    let rest = rest.trim_start();
    if let Some(after_q) = rest.strip_prefix('"') {
        let end = after_q.find('"')?;
        let v = &after_q[..end];
        return (!v.is_empty()).then(|| v.to_string());
    }
    let v: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != '#').collect();
    (!v.is_empty()).then_some(v)
}

/// Strip a single surrounding double-quote pair from a `.host-software` value token: `"main"`
/// yields `main`, a bare token is returned unchanged. `.host-software` value lines are bare by
/// convention (only the `[software "<name>"]` subsection name is quoted), but a heavily-quantized
/// operator writes `worktrees = "main"`; without this the quotes leak into refs, paths, hashes,
/// and URLs (issue #6). A lone, unbalanced, or interior quote (`"main`, `"`, `a"b`, `"a"b"`) is a
/// mis-authored value, reported as `None` so the caller can fail per its own discipline. Unlike
/// `stamp_value_after_eq` this keeps the whole token (multi-token fields unquote per token) and
/// does not strip a trailing `# comment` — it only concerns quotes.
fn unquote_recipe_token(tok: &str) -> Option<String> {
    if let Some(rest) = tok.strip_prefix('"') {
        let inner = rest.strip_suffix('"')?; // `"main` (unbalanced) / `"` (lone) -> None
        if inner.contains('"') {
            return None; // `"a"b"` — an interior quote is malformed
        }
        return Some(inner.to_string());
    }
    if tok.contains('"') {
        return None; // `main"` — a stray trailing/interior quote with no opening wrapper
    }
    Some(tok.to_string())
}

/// Every value for `key` (`key = "v"` or `key = v …`), in file order. The key must
/// be followed (after optional spaces) by `=`, so `revision` never matches
/// `revisionx`. Comment- and quote-tolerant. Multi-valued for repeated keys
/// (e.g. `applied`).
fn stamp_values(text: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix(key) else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
        if let Some(v) = stamp_value_after_eq(rest) {
            out.push(v);
        }
    }
    out
}

/// The first value for `key`; empty counts as absent.
fn stamp_field(text: &str, key: &str) -> Option<String> {
    stamp_values(text, key).into_iter().next()
}

/// The applied entry ids: the first token of each `applied = …` line (the rest of
/// the line is provenance — `recorded=… via=…` — written by `--record`).
#[cfg(test)]
fn applied_ids(text: &str) -> Vec<String> {
    stamp_values(text, "applied")
}

/// The `baseline` ledger entry id (every ledger entry at-or-before its position is
/// applied), if the stamp carries one.
fn baseline_field(text: &str) -> Option<String> {
    stamp_field(text, "baseline")
}

/// Whether ledger entry `id` is applied: explicitly in the `applied` set, or at/before
/// the `baseline`'s position in `ledger_ids` (file order). Pure position/membership —
/// no git ancestry (plan/0022 v2: ledger SHAs are linear-chain artifacts, and some
/// are orphaned from HEAD, so `merge-base` is the wrong and unreliable basis).
fn entry_applied(id: &str, ledger_ids: &[String], baseline: Option<&str>, applied: &[String]) -> bool {
    if applied.iter().any(|a| a == id) {
        return true;
    }
    let Some(base) = baseline else { return false };
    let pos = |x: &str| ledger_ids.iter().position(|e| e == x);
    matches!((pos(id), pos(base)), (Some(i), Some(b)) if i <= b)
}

/// Replace the first `key = …` line's value, preserving every other line (so `name`
/// and unknown keys survive — `stamp_body` drops them). Inserts the line if absent.
/// The all-field-preserving writer the re-stamp/baseline-advance paths use.
fn set_stamp_field(text: &str, key: &str, value: &str) -> String {
    let trailing = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut replaced = false;
    for l in lines.iter_mut() {
        let after = l.trim_start().strip_prefix(key).map(str::trim_start);
        if matches!(after, Some(a) if a.starts_with('=')) {
            *l = format!("{key} = \"{value}\"");
            replaced = true;
            break;
        }
    }
    if !replaced {
        lines.push(format!("{key} = \"{value}\""));
    }
    let mut s = lines.join("\n");
    if trailing {
        s.push('\n');
    }
    s
}

/// Append a raw line to a stamp (an `applied = …` provenance line), preserving
/// everything before it. Append-only — never rewrites a prior line, so a fumbled
/// `--record` can re-list but never corrupt an existing claim.
fn append_stamp_line(text: &str, line: &str) -> String {
    let mut s = text.to_string();
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s.push_str(line);
    s.push('\n');
    s
}

/// Remove the `applied = <id> …` lines whose id is in `ids`, preserving every other
/// line. Used by baseline-advance to drop ids it has absorbed into the contiguous
/// baseline — a deliberate compaction (the entries stay applied via the baseline),
/// not a silent rewrite of a live claim.
fn remove_applied_lines(text: &str, ids: &[String]) -> String {
    let trailing = text.ends_with('\n');
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| {
            let is_absorbed = l
                .trim_start()
                .strip_prefix("applied")
                .and_then(|r| r.trim_start().strip_prefix('='))
                .and_then(stamp_value_after_eq)
                .is_some_and(|id| ids.iter().any(|x| x == &id));
            !is_absorbed
        })
        .collect();
    let mut s = kept.join("\n");
    if trailing {
        s.push('\n');
    }
    s
}

/// Read a MADR header field (`- Status: accepted`, `Scope: x`) from a decision body.
fn decision_field(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim_start();
        let l = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).unwrap_or(l);
        if let Some(rest) = l.strip_prefix(key) {
            if let Some(v) = rest.strip_prefix(':') {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Anti-ouroboros gate: a live (accepted) decision needs a `Scope:` and must not be
/// methodology. Retired decisions (superseded/deprecated/rejected/proposed) pass. A
/// `Scope:` that names `host-template` is also flagged (plan/0036, the reconcile arm's
/// sibling): the decision authored a rule that now lives in the spine, so it must be
/// superseded there rather than left `accepted` (the `call/0017` decision-status drift).
fn decision_scope_problem(text: &str) -> Option<&'static str> {
    let status = decision_field(text, "Status").unwrap_or_default();
    if !status.to_ascii_lowercase().starts_with("accepted") {
        return None;
    }
    match decision_field(text, "Scope") {
        None => Some("accepted decision is missing a `Scope:` header"),
        Some(s) if s.eq_ignore_ascii_case("methodology") => {
            Some("accepted decision is `Scope: methodology` — methodology belongs in the template spine; supersede it there")
        }
        Some(s) if s.to_ascii_lowercase().contains("host-template") => {
            Some("accepted decision names `host-template` in its `Scope:` — its rule is now spine-resident; set `Status: superseded by the spine`")
        }
        Some(_) => None,
    }
}

/// Migration case from what governance a repo already carries.
fn classify_case(has_stamp: bool, has_claude: bool) -> &'static str {
    if has_stamp {
        "c"
    } else if has_claude {
        "b"
    } else {
        "a"
    }
}

/// Root-level build manifests that mark a directory as a software repository. A
/// host root never carries these — its software lives in gitignored worktrees
/// recorded in `.host-software` — so finding one at first adoption means the
/// target is software being adopted in place, which the methodology forbids.
const SOFTWARE_MANIFESTS: &[&str] = &[
    "Cargo.toml", "package.json", "go.mod", "pyproject.toml", "setup.py",
    "pom.xml", "build.gradle", "build.gradle.kts", "Gemfile", "composer.json",
    "CMakeLists.txt", "mix.exs", "Package.swift",
];

/// The first root-level software manifest present, if any.
fn software_manifest(root: &Path) -> Option<&'static str> {
    SOFTWARE_MANIFESTS.iter().copied().find(|m| root.join(m).is_file())
}

/// First-adoption guard. Returns the detected manifest when the target carries
/// software but is neither stamped (case c, already a host) nor already managing
/// software via a `.host-software` recipe — i.e. an attempt to adopt a software
/// repository in place, which the methodology refuses. `None` means proceed.
fn adopt_in_place_refusal(root: &Path) -> Option<&'static str> {
    if root.join(STAMP).is_file() || root.join(SOFTWARE).is_file() {
        return None;
    }
    software_manifest(root)
}

/// Today's date as `YYYY-MM-DD` (UTC). Deterministic formatting via
/// [`civil_from_days`]; only the clock read is impure.
fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}

/// The adoption rename dictionary (transient scaffold; removed after the remap).
const REMAP: &str = ".host-remap";
/// The sanctioned-vocabulary file `host-lint` reads; we honour it so a token that
/// is allow-listed there is not reported as undispositioned.
const ALLOW: &str = ".host-lint-allow";
/// The path-ignore file `host-lint --all` reads; we honour it so paths excluded
/// from the audit (the append-only record) are also out of scope for the remap.
const IGNORE: &str = ".host-lintignore";
/// The software-under-test recipe (`call/0010`): one `[software "<name>"]` stanza
/// per component — a bare store plus its canonical worktree at `pin`.
const SOFTWARE: &str = ".host-software";
/// Spec file extensions (behaviour `.allium`, timing `.tla`/`.cfg`). `host-lint`'s
/// scannable set omits these, so the remap skipped spec-internal cross-references
/// silently (issue #7); remap treats them as targets too — the rewrite is map-only,
/// so plain-text spec bodies are safe to run the declared substitutions over.
const SPEC_EXTS: [&str; 3] = ["allium", "tla", "cfg"];

/// One sanctioned substitution: match `old` (case-insensitive, word-bounded),
/// replace with `new` verbatim. The human supplies `new`; the tool never coins it.
struct Rule {
    old_lc: String,
    new: String,
}

/// `remap --check|--apply <dir> [--dry-run]` — apply a declared `.host-remap`
/// dictionary deterministically. The dictionary is the only source of new names,
/// so the rewrite is map-only by construction: no token outside it is ever
/// touched (no fabrication, no drift across files). `--check` reports the tells
/// that would remain (undispositioned — they need a dictionary or allow entry);
/// `--apply` writes the substitutions, refusing unless the git tree is clean so
/// the prior commit is the verbatim archive (`CLAUDE.md` §6).
fn remap(args: &[String]) {
    let mut mode: Option<&str> = None;
    let mut dry = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => mode = Some("check"),
            "--apply" => mode = Some("apply"),
            "--dry-run" => dry = true,
            _ => pos.push(a),
        }
    }
    let Some(dir) = pos.first() else {
        eprintln!("host-lifecycle remap <--check|--apply> <dir> [--dry-run]");
        process::exit(2);
    };
    let Some(mode) = mode else {
        eprintln!("host-lifecycle remap needs --check or --apply");
        process::exit(2);
    };
    let root = Path::new(dir.as_str());
    if !root.is_dir() {
        eprintln!("host-lifecycle: not a directory: {}", root.display());
        process::exit(2);
    }
    let rules = load_remap(root);
    let allow = remap_allow(root);
    // `.host-lint-allow` is a deprecated alias for the LEXICON (host-lifecycle#13); note it
    // when present so an adopter migrates the declaration to host-lint's LEXICON.
    if !load_allow(root).is_empty() {
        eprintln!("host-lifecycle: note: {ALLOW} is a deprecated alias; declare sanctioned tokens in the LEXICON instead");
    }
    let ignore = load_ignore(root);
    match mode {
        "check" => remap_check(root, &rules, &allow, &ignore),
        "apply" => remap_apply(root, &rules, &ignore, dry),
        _ => unreachable!(),
    }
}

/// Parse `.host-remap`: `old => new` per line, `#` comments and blanks ignored.
/// Sorted longest-`old`-first so `Phase 5.0` is consumed before `Phase 5`. An absent
/// dictionary yields no rules (issue #7): the caller then audits with zero rules rather
/// than erroring, so an empty rule set never stands in for a skipped scan.
fn load_remap(root: &Path) -> Vec<Rule> {
    let text = match fs::read_to_string(root.join(REMAP)) {
        Ok(t) => t,
        // A missing dictionary is a fail-safe no-op, not an error. Any other read fault
        // (a permission denial, a bad encoding) still exits loudly.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            eprintln!("host-lifecycle: cannot read {REMAP}: {e}");
            process::exit(2);
        }
    };
    let mut rules = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some((old, new)) = t.split_once(" => ") else {
            eprintln!("host-lifecycle: {REMAP}:{}: expected `old => new`", i + 1);
            process::exit(2);
        };
        let (old, new) = (old.trim(), new.trim());
        if old.is_empty() {
            eprintln!("host-lifecycle: {REMAP}:{}: empty match side", i + 1);
            process::exit(2);
        }
        rules.push(Rule {
            old_lc: old.to_ascii_lowercase(),
            new: new.to_string(),
        });
    }
    rules.sort_by_key(|r| std::cmp::Reverse(r.old_lc.len()));
    rules
}

/// The repo's sanctioned vocabulary (`.host-lint-allow`), ASCII-lowercased — same
/// format `host-lint` reads. Absent file → empty (no exemptions).
fn load_allow(root: &Path) -> Vec<String> {
    let text = match fs::read_to_string(root.join(ALLOW)) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_ascii_lowercase())
        .collect()
}

/// The sanctioned vocabulary `remap` honours: host-lint's LEXICON (canonical) merged
/// with the legacy `.host-lint-allow` alias (host-lifecycle#13). Unifying the two means
/// a token declared for host-lint is not a tell to remap, and one declared to remap is
/// not a tell to host-lint — one source, not two that drift.
fn remap_allow(root: &Path) -> Vec<String> {
    let mut allow = host_lint::load_lexicon(root).phrases_lc;
    allow.extend(load_allow(root));
    allow
}

/// The repo's `.host-lintignore` patterns (paths excluded from the audit), so the
/// remap leaves the same paths alone. Absent file → empty.
fn load_ignore(root: &Path) -> Vec<String> {
    match fs::read_to_string(root.join(IGNORE)) {
        Ok(t) => t
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Replace every word-bounded, case-insensitive occurrence of `old_lc` in `s`
/// with `new`. The boundary requirement (a non-alphanumeric neighbour or a string
/// edge) keeps a rule specific: `phase 1` rewrites `phase 1` but not `phase 12`.
fn replace_bounded_ci(s: &str, old_lc: &str, new: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if !old_lc.is_empty() && lower[i..].starts_with(old_lc) {
            let end = i + old_lc.len();
            let left_ok = i == 0 || !lb[i - 1].is_ascii_alphanumeric();
            let right_ok = end == lb.len() || !lb[end].is_ascii_alphanumeric();
            if left_ok && right_ok {
                out.push_str(new);
                i = end;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Apply every rule to one line, longest-`old` first (the order `load_remap` set).
fn apply_rules(line: &str, rules: &[Rule]) -> String {
    let mut cur = line.to_string();
    for r in rules {
        cur = replace_bounded_ci(&cur, &r.old_lc, &r.new);
    }
    cur
}

/// The info string of a markdown fence line (the text after ``` / ~~~), with the
/// marker char and run length, or None if not a fence: 3+ backticks or tildes with at
/// most 3 leading spaces. Replicates host-lint's scan-side rule so `remap --apply`
/// skips the exact `host-lint:ignore` blocks the scan skips (host-lifecycle#12),
/// self-contained so M2 needs no host-lint change.
fn fence_info(line: &str) -> Option<(char, usize, &str)> {
    if line.chars().take_while(|c| *c == ' ').count() >= 4 {
        return None;
    }
    let t = line.trim_start();
    let marker = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = t.chars().take_while(|c| *c == marker).count();
    if run < 3 {
        return None;
    }
    Some((marker, run, t[run..].trim()))
}

/// Apply the rules across a whole file's text, preserving exact line structure
/// (LF/CRLF and whether the file ends in a newline). In a markdown source a
/// `host-lint:ignore` fenced block is emitted verbatim (its lines and fences), so
/// `remap --apply` never rewrites inside the box the scan already skips
/// (host-lifecycle#12); a regular code fence stays substituted, matching the scan, and
/// a non-markdown source has no fence semantics.
fn apply_text(text: &str, rules: &[Rule], markdown: bool) -> String {
    let mut out = String::with_capacity(text.len());
    // The open `host-lint:ignore` fence's marker char and run length, or None outside.
    let mut ignore_fence: Option<(char, usize)> = None;
    for chunk in text.split_inclusive('\n') {
        let (body, nl) = match chunk.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (chunk, ""),
        };
        let (body, cr) = match body.strip_suffix('\r') {
            Some(b) => (b, "\r"),
            None => (body, ""),
        };
        let emit = if !markdown {
            apply_rules(body, rules)
        } else if let Some((mch, mlen)) = ignore_fence {
            // Inside the fence (its close line included): emit verbatim; a bare
            // same-marker fence at least as long closes it (CommonMark).
            if let Some((c, len, info)) = fence_info(body) {
                if info.is_empty() && c == mch && len >= mlen {
                    ignore_fence = None;
                }
            }
            body.to_string()
        } else if let Some((c, len, info)) = fence_info(body) {
            if info == "host-lint:ignore" {
                ignore_fence = Some((c, len));
                body.to_string() // the open fence line is skipped by the scan
            } else {
                apply_rules(body, rules) // a regular fence is scanned, so substituted too
            }
        } else {
            apply_rules(body, rules)
        };
        out.push_str(&emit);
        out.push_str(cr);
        out.push_str(nl);
    }
    out
}

/// Submodule paths from `.gitmodules`, so the walk never descends into another
/// repo (the software submodules are out of scope for a host remap).
fn submodule_paths(root: &Path) -> Vec<String> {
    let text = match fs::read_to_string(root.join(".gitmodules")) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("path")
                .and_then(|r| r.trim_start().strip_prefix('='))
                .map(|v| v.trim().to_string())
        })
        .collect()
}

/// Collect tracked-ish text files under `root`, skipping VCS/build dirs and any
/// submodule path.
fn collect_files(dir: &Path, root: &Path, subs: &[String], out: &mut Vec<PathBuf>) {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        // Take the type from the DirEntry, which does NOT follow symlinks, and skip symlinks
        // (issue #24) as the sibling `tracked_markdown` does: a symlink out of the tree must
        // not be classified by its target or descended into.
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if matches!(name.as_str(), ".git" | "target" | "node_modules" | "vendor") {
                continue;
            }
            let rel = p
                .strip_prefix(root)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"));
            if let Some(rel) = &rel {
                // The materialized Where room (plan/0029): `software/<component>/…` holds
                // every component's full source — never walked by the naming audit / remap.
                if rel == "software" || subs.iter().any(|s| s == rel) {
                    continue;
                }
            }
            collect_files(&p, root, subs, out);
        } else if ft.is_file() {
            out.push(p);
        }
    }
}

/// A file the remap should touch: scannable by `host-lint` or a spec file, not a CI
/// file, and not one of our own control files (the dictionary, the allow file, the
/// stamp).
fn is_target(p: &Path) -> bool {
    let s = p.to_string_lossy();
    if is_ci_file(&s) {
        return false;
    }
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if matches!(name.as_str(), REMAP | ALLOW | STAMP) {
        return false;
    }
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    is_scannable(ext) || is_spec_ext(ext)
}

/// A spec extension the remap rewrites even though `host-lint` does not scan it.
fn is_spec_ext(ext: &str) -> bool {
    SPEC_EXTS.contains(&ext)
}

/// `--check`: apply the dictionary in memory, then run `host-lint` over the result
/// (honouring `.host-lint-allow`) and report every tell that remains — these are
/// undispositioned and need a dictionary or allow entry. Exit 1 on a remaining
/// flag, 3 on a remaining warning, 0 when clean.
fn remap_check(root: &Path, rules: &[Rule], allow: &[String], ignore: &[String]) {
    process::exit(remap_check_code(root, rules, allow, ignore));
}

/// The scan-and-decide core of `--check`, returning the exit code (`1` on any flag, `3` on
/// any warn, else `0`) so it is unit-testable off the `process::exit` boundary. An empty
/// `rules` still walks and scans every target, so a missing or empty `.host-remap` audits
/// rather than reporting a hollow clean (issue #7).
fn remap_check_code(root: &Path, rules: &[Rule], allow: &[String], ignore: &[String]) -> i32 {
    let subs = submodule_paths(root);
    let mut files = Vec::new();
    collect_files(root, root, &subs, &mut files);
    files.sort();

    let mut scanned = 0usize;
    let mut changed = 0usize;
    let mut specs = 0usize;
    let mut remaining: Vec<Match> = Vec::new();
    for f in &files {
        if !is_target(f) {
            continue;
        }
        let src = f.strip_prefix(root).unwrap_or(f).to_string_lossy().replace('\\', "/");
        if path_ignored(&src, ignore) {
            continue;
        }
        let Ok(content) = fs::read_to_string(f) else {
            continue;
        };
        scanned += 1;
        if is_spec_ext(f.extension().and_then(|e| e.to_str()).unwrap_or("")) {
            specs += 1;
        }
        let applied = apply_text(&content, rules, src.to_lowercase().ends_with(".md"));
        if applied != content {
            changed += 1;
        }
        scan_text_with_allow(&applied, &src, allow, &mut remaining);
    }
    for m in &remaining {
        let kind = if m.severity == Severity::Warn { "warning" } else { "tell" };
        println!("{}:{}: {kind}: {} ({})", m.file, m.line, m.text, m.term);
    }
    let outcome = if remaining.is_empty() { "clean" } else { "author a .host-remap or allow entry" };
    println!(
        "-- {} rule(s); {changed}/{scanned} file(s) would change ({specs} spec file(s) scanned); {} undispositioned tell(s) remain; {outcome}",
        rules.len(),
        remaining.len()
    );
    if remaining.iter().any(|m| m.severity == Severity::Flag) {
        return 1;
    }
    if remaining.iter().any(|m| m.severity == Severity::Warn) {
        return 3;
    }
    0
}

/// `--apply`: write the substitutions. Refuses unless the git tree is clean, so
/// the prior commit archives the originals verbatim (`CLAUDE.md` §6). `--dry-run`
/// previews without the guard and without writing.
fn remap_apply(root: &Path, rules: &[Rule], ignore: &[String], dry: bool) {
    // An empty dictionary renames nothing, so it is a fail-safe no-op that writes nothing and
    // needs no clean tree — return before the clean-git guard rather than erroring (issue #7).
    if rules.is_empty() {
        println!(
            "remap --apply: no rules in {REMAP}; nothing to rename (0 files {})",
            if dry { "would change (dry-run)" } else { "changed" }
        );
        return;
    }
    if !dry {
        require_clean_git(root);
    }
    let subs = submodule_paths(root);
    let mut files = Vec::new();
    collect_files(root, root, &subs, &mut files);
    files.sort();

    let mut changed = 0usize;
    let mut specs = 0usize;
    for f in &files {
        if !is_target(f) {
            continue;
        }
        let rel = f.strip_prefix(root).unwrap_or(f).to_string_lossy().replace('\\', "/");
        if path_ignored(&rel, ignore) {
            continue;
        }
        let Ok(content) = fs::read_to_string(f) else {
            continue;
        };
        let applied = apply_text(&content, rules, rel.to_lowercase().ends_with(".md"));
        if applied == content {
            continue;
        }
        if is_spec_ext(f.extension().and_then(|e| e.to_str()).unwrap_or("")) {
            specs += 1;
        }
        if dry {
            println!("would remap {rel}");
        } else if let Err(e) = fs::write(f, &applied) {
            eprintln!("host-lifecycle: cannot write {}: {e}", f.display());
            process::exit(2);
        } else {
            println!("remap  {rel}");
        }
        changed += 1;
    }
    println!(
        "-- {changed} file(s) {} ({specs} spec file(s) included)",
        if dry { "would change (dry-run)" } else { "remapped" }
    );
    if !dry {
        eprintln!("note: confirm with `host-lint --all`; the prior commit is the verbatim archive (§6).");
    }
}

/// Refuse `--apply` unless `git status --porcelain` is empty: the prior commit
/// must hold the originals before we overwrite them.
fn require_clean_git(root: &Path) {
    match process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain"])
        .output()
    {
        Ok(o) if o.status.success() => {
            if !o.stdout.is_empty() {
                eprintln!(
                    "host-lifecycle: working tree not clean — commit first so the prior commit archives the originals (§6). Refusing to apply."
                );
                process::exit(2);
            }
        }
        _ => {
            eprintln!(
                "host-lifecycle: not a git repo or git unavailable; --apply needs a clean git tree as the archive. Use --dry-run to preview."
            );
            process::exit(2);
        }
    }
}

/// One software component from `.host-software`: a bare object store plus its
/// canonical worktree at `pin` and any parallel worktrees (`call/0010`).
#[derive(Clone)]
struct Software {
    name: String,
    url: String,
    pin: String,
    /// The canonical worktree's branch (plan/0029): the pin is checked out at
    /// `software/<name>/<branch>/` on this branch, reset to `pin`. Defaults to `main`.
    branch: String,
    /// Bare `worktrees = <branch> ...` form: a parallel line per branch, materialized
    /// at `software/<name>/<branch>/` (the branch is the path key; plan/0029).
    worktrees: Vec<String>,
    /// Explicit `worktree = <branch> <pin> [store=<path>] [host=<os>]` form: a parallel
    /// line on its own branch at its own pin, faithfully reproducible by `--materialize`.
    /// The path is derived from the branch (`software/<name>/<branch>/`); a `store=`
    /// line lives off-tree with that path as the in-tree handle.
    lines: Vec<Worktree>,
    /// Reproducible-build provenance (issue #10), all optional:
    /// `build`/`toolchain` — the recorded deterministic recipe (run by `--verify-build`);
    /// `deploy` — which line (canonical name or a `worktree` dir) is the deployed line;
    /// `artifact` — `<path> <sha256>` the deployed artifact must hash to (attestation);
    /// `repro_exempt` — `call/NNNN` authorizing a not-yet-reproducible migrated build.
    build: Option<String>,
    toolchain: Option<String>,
    deploy: Option<String>,
    artifact: Option<(String, String)>,
    repro_exempt: Option<String>,
    /// `hooks` — a commit-hook dispatch script (relative to the canonical
    /// worktree) that `--install-hooks` copies into `.git/hooks` as both
    /// `pre-commit` and `commit-msg`, alongside the verified deploy artifact.
    hooks: Option<String>,
    /// `deps-bundle = <url> <sha256>` (plan/0032) — a pinned, hash-verified vendored
    /// dependency bundle. When set, `--verify-build`/`release` download it, verify the
    /// sha (the provenance half of the hermeticity gate), stage it into the build tree,
    /// and build under `--network none` (the egress half). The tarball ships `vendor/`
    /// plus a `vendor-config.toml` source-replacement snippet.
    deps_bundle: Option<(String, String)>,
    /// Explicit per-platform builds (issue #1): `[build "<name>" "<platform>"]`
    /// subsections, each a distinct toolchain/artifact of the *same* source `pin`.
    /// When non-empty, these replace the flat `build`/`artifact`/… fields above;
    /// when empty, the flat fields form the single default build (back-compat).
    builds: Vec<PlatformBuild>,
}

/// An explicit parallel worktree: a tree checked out on `branch` at `pin`, located at
/// `software/<name>/<branch>/` (the in-structure handle, under the host root). When
/// `store` is set the git worktree physically lives there — possibly off-tree /
/// off-filesystem — and the in-tree path is materialized as a symlink/junction to it,
/// so an agent editing it writes the files under test (issue #2). `host`, when set,
/// gates the line to one OS (`std::env::consts::OS`), mirroring a build's `attest_host`:
/// off-platform the line is skipped by `--materialize`/`--check` rather than reported
/// missing.
#[derive(Clone)]
struct Worktree {
    branch: String,
    pin: String,
    store: Option<String>,
    host: Option<String>,
}

/// One platform's build of a component, sharing the component's `url`+`pin` but
/// carrying its own recipe and artifact (issue #1). `attest_host` names the OS
/// (`std::env::consts::OS`: `linux`/`windows`/`macos`) on which this build is
/// reproducible; `--check`/`--verify-build` skip it on any other host, the way an
/// exempt component is skipped, rather than failing.
#[derive(Clone)]
struct PlatformBuild {
    platform: String,
    build: Option<String>,
    toolchain: Option<String>,
    deploy: Option<String>,
    artifact: Option<(String, String)>,
    repro_exempt: Option<String>,
    attest_host: Option<String>,
}

/// A component's effective builds for provenance: borrows either the explicit
/// per-platform builds or, when there are none, the flat single-build fields.
struct BuildView<'a> {
    platform: Option<&'a str>,
    build: Option<&'a str>,
    toolchain: Option<&'a str>,
    deploy: Option<&'a str>,
    artifact: Option<&'a (String, String)>,
    repro_exempt: Option<&'a str>,
    attest_host: Option<&'a str>,
}

impl Software {
    /// The builds to attest: the explicit `[build …]` subsections, or a single
    /// default view over the flat fields (no `attest-host`, so it attests on any
    /// host — the pre-issue-#1 behaviour).
    fn builds_view(&self) -> Vec<BuildView<'_>> {
        if self.builds.is_empty() {
            return vec![BuildView {
                platform: None,
                build: self.build.as_deref(),
                toolchain: self.toolchain.as_deref(),
                deploy: self.deploy.as_deref(),
                artifact: self.artifact.as_ref(),
                repro_exempt: self.repro_exempt.as_deref(),
                attest_host: None,
            }];
        }
        self.builds
            .iter()
            .map(|b| BuildView {
                platform: Some(&b.platform),
                build: b.build.as_deref(),
                toolchain: b.toolchain.as_deref(),
                deploy: b.deploy.as_deref(),
                artifact: b.artifact.as_ref(),
                repro_exempt: b.repro_exempt.as_deref(),
                attest_host: b.attest_host.as_deref(),
            })
            .collect()
    }
}

/// Narrow a recipe to a single `--item <name>[@<branch>]` (plan/0029). `<name>` selects
/// the component; an optional `@<branch>` narrows to just that worktree — the branch
/// becomes the operation's canonical, at the matching pin (a parallel line's own pin,
/// else the component pin). Exits if the named component is absent.
fn filter_item(recipe: Vec<Software>, spec: &str) -> Vec<Software> {
    let (name, branch) = match spec.split_once('@') {
        Some((n, b)) => (n, Some(b)),
        None => (spec, None),
    };
    let Some(s) = recipe.into_iter().find(|s| s.name == name) else {
        eprintln!("host-lifecycle: --item: no component named `{name}` in {SOFTWARE}");
        process::exit(2);
    };
    match branch {
        None => vec![s],
        Some(branch) => {
            let pin = s
                .lines
                .iter()
                .find(|w| w.branch == branch)
                .map(|w| w.pin.clone())
                .unwrap_or_else(|| s.pin.clone());
            let mut narrowed = s;
            narrowed.branch = branch.to_string();
            narrowed.pin = pin;
            narrowed.worktrees = Vec::new();
            narrowed.lines = Vec::new();
            vec![narrowed]
        }
    }
}

/// `software --materialize|--check <dir>` — realise the `.host-software` recipe.
/// `--materialize` clones each `<name>/.bare` bare store (with a `.git` gitdir-link
/// beside it, call/0039) and adds the canonical worktree `<name>/<branch>/` at its
/// `pin` (plus any parallel worktrees), idempotently —
/// it skips what already exists. `--check` verifies each canonical worktree is at
/// its recorded pin: the audit that replaces a submodule gitlink's `git submodule
/// status`.
/// `.host-lintignore` patterns (gitignore-lite: one per line, `#`/blank skipped) — the
/// same exclusion file host-lint's `--docs`/`--all` walk honors (e.g. the append-only
/// `MEMORY.md`).
fn load_lintignore(root: &Path) -> Vec<String> {
    match fs::read_to_string(root.join(".host-lintignore")) {
        Ok(c) => c
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The prose-hygiene audit (plan/0030 D4): run host-lint's `--docs` engine in-process via
/// the linked `host_lint` crate, so the verdict is byte-identical to `host-lint --docs`
/// without needing host-lint on PATH (this host's host-lint is embedded Where software).
/// `load_lexicon` supplies the repo's LEXICON masking phrases and `run_docs` performs the
/// shared `git ls-files` walk over tracked `.md` (honoring `.host-lintignore`, skipping
/// symlinks and non-files), masking each declared phrase before detection — so a domain
/// noun a project declares in its LEXICON clears the same trope here as at the CLI, and the
/// embedded engine cannot drift from the standalone one (host-lifecycle#2). Returns the
/// accumulated matches, or an error if the repo cannot be walked.
fn prose_audit(root: &Path) -> Result<Vec<Match>, String> {
    let allow = host_lint::load_lexicon(root).phrases_lc;
    let ignore = load_lintignore(root);
    host_lint::run_docs(root, &allow, &ignore)
}

/// `prose <dir>`: the repo-wide prose-hygiene recheck (plan/0030 D4), the portable form of
/// `host-lint --docs`. It exits with host-lint's own verdict — a `Flag` exits 1, a `Warn`
/// exits 3, else 0 — so chaining it after `validate` in the `verify` phase `recheck =`
/// re-opens the verify receipt as a HAZARD when a doc regresses to prose slop (the spine's
/// standing prose rule, now enforced by the gate). Advisory `Note`-tier diagnoses do not
/// block, exactly as the `--docs` clean-to-zero bar terminates.
fn prose(args: &[String]) {
    let dir = args.iter().find(|a| !a.starts_with("--")).map(String::as_str).unwrap_or(".");
    let root = match fs::canonicalize(Path::new(dir)) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    let matches = match prose_audit(&root) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("host-lifecycle: prose: {e}");
            process::exit(2);
        }
    };
    let flags = matches.iter().filter(|m| m.severity == Severity::Flag).count();
    let warns = matches.iter().filter(|m| m.severity == Severity::Warn).count();
    if flags + warns == 0 {
        println!("prose: clean (authored markdown carries no flagging or warning tropes)");
        return;
    }
    for m in &matches {
        match m.severity {
            Severity::Flag => println!("{}:{}: {} ({})", m.file, m.line, m.text, m.term),
            Severity::Warn => println!("{}:{}: warning: {} ({})", m.file, m.line, m.text, m.term),
            Severity::Note => {}
        }
    }
    eprintln!("host-lifecycle: prose regression — {flags} flag(s), {warns} warn(s) in authored docs");
    process::exit(if flags > 0 { 1 } else { 3 });
}

// ---- Reconcile (plan/0036) -------------------------------------------------
//
// The second arm of the "grows by reflective practice" doctrine. The first arm
// (gather) catches emergent tells the lane misses; reconcile catches a project's
// own restatement of methodology that a spine move staled. Scope is the annotated
// set, not a judgment: a restatement that must stay carries an inline
// `<!-- host-reconcile: KIND -->` annotation declaring an assertion the tool checks
// against a source of truth (the project's components and verifier drivers in
// `.host-software` and the fixed layout), so the check is operable at the weak-agent bar. The one-time
// discovery of the drift is a human audit; the annotation makes recurrence mechanical.

/// The reconcile-assertion kinds: the machine-checkable restatement shapes the
/// reconcile arm verifies. A doc annotation `<!-- host-reconcile: KIND -->` and an
/// `UPGRADING` `restates =` field both name a kind from this set.
const RECONCILE_KINDS: [&str; 4] = ["components", "verification", "where-root", "spec-path"];

/// The inline annotation marker; the kind follows up to the closing `-->`.
const RECONCILE_MARK: &str = "<!-- host-reconcile:";

/// The concepts an entrance can be held complete against, each with a structured home the
/// tool reads: `phases` (the manifest), `tools` (the `.host-software` drivers plus the
/// lifecycle engine), and `stamp` (the `.host` stamp format). A closed vocabulary, so an
/// unknown concept in a `restates` value is a loud parse error rather than a silent skip
/// (plan/0043). `tools` corresponds to reconcile's verifiers.
const ENTRANCE_CONCEPTS: [&str; 3] = ["phases", "tools", "stamp"];

/// Which concepts an entrance keeps complete. `All` is the `restates = true` sentinel (the
/// front-door case, every concept), so a full front door never types an enumeration; `Set`
/// is a validated subset of `ENTRANCE_CONCEPTS`.
#[derive(Clone)]
enum Restates {
    All,
    Set(Vec<String>),
}

impl Restates {
    /// Whether the entrance is held complete against `concept`.
    fn checks(&self, concept: &str) -> bool {
        match self {
            Restates::All => true,
            Restates::Set(s) => s.iter().any(|c| c == concept),
        }
    }
}

/// The single-file entrance (plan/0043): the one document held to the spine, declared in a
/// global-singleton `[entrance]` stanza. `member` is the `.host-software` member it belongs
/// to (set apart from `components`); `document` is the file within that member's worktree
/// (default `README.md`, so a `SKILL.md` or a landing page is reached by path); `restates`
/// is the concept set it keeps complete.
#[derive(Clone)]
struct Entrance {
    member: String,
    document: String,
    restates: Restates,
}

/// A project's own concept facts (plan/0039), read from its `.host-software`, never the
/// spine. `components` are the project's `[software "<name>"]` members minus the entrance
/// member (set apart from the host-* tools); `drivers` are the verifiers, the
/// `[verification] drivers = ...` list; `entrance` is the declared `[entrance]` stanza, if
/// any; `problems` are the entrance-stanza parse errors (a second stanza, an unknown concept,
/// a member that is not declared). Empty when `.host-software` is absent or carries no such
/// data, so the matching assertions are unverifiable and skipped, never a false HAZARD.
/// Project-local by design: the spine manifest is phases only (`manifest --check`), so no
/// adopter inherits another project's facts.
#[derive(Default)]
struct ProjectFacts {
    components: Vec<String>,
    drivers: Vec<String>,
    entrance: Option<Entrance>,
    problems: Vec<String>,
}

/// Parse `.host-software` for a project's concept facts. `components` = every
/// `[software "<name>"]` member except the entrance member; `drivers` = the
/// `[verification] drivers = ...` list; `entrance` = the `[entrance]` stanza, a global
/// singleton naming `member`, an optional `document` (default `README.md`), and a `restates`
/// concept set (default every concept). The legacy per-member `entrance = true` (and the
/// older `front-door = true`) is retired (plan/0043): a surviving one is a `problems` entry,
/// never the entrance. Mirrors `parse_software`'s git-config style; unknown sections and keys are ignored
/// (forward-compatible). A member that forgets every marker is counted a component, the
/// fail-safe: coverage then over-reports rather than hiding it. `problems` records a second
/// stanza, an unknown concept, or a named member that is not declared (plan/0043).
fn parse_project_facts(text: &str) -> ProjectFacts {
    // Split a list value and strip a `"..."` wrapper from each token; a mis-quoted token is kept
    // raw and surfaces downstream (issue #6), matching this parser's tolerant discipline (it
    // collects problems rather than exiting the way `parse_software` does).
    let split = |v: &str| -> Vec<String> {
        v.split([',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| unquote_recipe_token(s).unwrap_or_else(|| s.to_string()))
            .collect()
    };
    let mut members: Vec<String> = Vec::new();
    let mut drivers: Vec<String> = Vec::new();
    let mut stanza_count = 0usize;
    let mut v_stanza_count = 0usize;
    let mut drivers_seen = false;
    let (mut e_member, mut e_document, mut e_restates) = (None::<String>, None::<String>, None::<String>);
    let mut section = ""; // "software" | "verification" | "entrance" | ""
    let mut current_member = String::new();
    let mut problems: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(name) = t.strip_prefix("[software \"").and_then(|r| r.strip_suffix("\"]")) {
            members.push(name.to_string());
            current_member = name.to_string();
            section = "software";
            continue;
        }
        if t.starts_with('[') {
            section = if t == "[entrance]" {
                stanza_count += 1;
                "entrance"
            } else if t.strip_prefix("[entrance").is_some_and(|r| r.starts_with(' ') || r.starts_with('"')) {
                // a sub-named `[entrance "x"]` is the wrong form (the stanza is anonymous);
                // parse it anyway so its keys are read, but flag it loudly.
                stanza_count += 1;
                problems.push(format!("the entrance stanza is `[entrance]`, not a sub-named section like `{t}`"));
                "entrance"
            } else if t == "[verification]" {
                v_stanza_count += 1;
                "verification"
            } else {
                ""
            };
            continue;
        }
        let Some((key, val)) = t.split_once('=') else { continue };
        let (key, val) = (key.trim(), val.trim());
        match (section, key) {
            // the legacy per-member marker is retired (plan/0043): a surviving one is a loud
            // problem, never silently accepted, so it cannot demote the entrance into a
            // component. Case-insensitive so a `True` is caught, not waved past.
            ("software", "entrance" | "front-door") if val.eq_ignore_ascii_case("true") => {
                problems.push(format!("the per-member `{key} = true` entrance marker is retired (plan/0043); declare an `[entrance]` stanza naming `{current_member}`"));
            }
            // issue #14: the verifier set is a singleton, so a repeated `drivers` key (last-wins)
            // or an empty value silently disarms the reconcile coverage check; flag both, the way
            // the `[entrance]` stanza guards `member`/`restates`.
            ("verification", "drivers") => {
                if drivers_seen {
                    problems.push("the `[verification]` stanza names `drivers` more than once".into());
                }
                drivers_seen = true;
                let parsed = split(val);
                if parsed.is_empty() {
                    problems.push("the `[verification]` `drivers` value is empty; name at least one rung-driver or omit the stanza".into());
                }
                drivers = parsed;
            }
            ("entrance", "member") => {
                if e_member.is_some() {
                    problems.push("the `[entrance]` stanza names `member` more than once".into());
                }
                e_member = Some(unquote_recipe_token(val).unwrap_or_else(|| val.to_string()));
            }
            ("entrance", "document") => e_document = Some(unquote_recipe_token(val).unwrap_or_else(|| val.to_string())),
            ("entrance", "restates") => e_restates = Some(unquote_recipe_token(val).unwrap_or_else(|| val.to_string())),
            _ => {}
        }
    }

    if v_stanza_count > 1 {
        problems.push(format!("more than one `[verification]` stanza ({v_stanza_count}); the verifier set is a global singleton"));
    }

    let entrance = if stanza_count > 0 {
        if stanza_count > 1 {
            problems.push(format!("more than one `[entrance]` stanza ({stanza_count}); the entrance is a global singleton"));
        }
        let restates = match e_restates.as_deref() {
            None => Restates::All,
            Some(s) if s.eq_ignore_ascii_case("true") => Restates::All,
            Some(list) => {
                let tokens = split(list);
                if tokens.is_empty() {
                    problems.push("the `[entrance]` `restates` value is empty; use `true` for every concept or name at least one".into());
                }
                for tk in &tokens {
                    if !ENTRANCE_CONCEPTS.contains(&tk.as_str()) {
                        problems.push(format!("the `[entrance]` restates an unknown concept `{tk}` (known: {})", ENTRANCE_CONCEPTS.join(", ")));
                    }
                }
                Restates::Set(tokens)
            }
        };
        Some(Entrance {
            member: e_member.unwrap_or_default(),
            document: e_document.unwrap_or_else(|| "README.md".into()),
            restates,
        })
    } else {
        None
    };

    if let Some(e) = &entrance {
        if e.member.is_empty() {
            problems.push("the `[entrance]` stanza names no `member`".into());
        } else if !members.iter().any(|m| m == &e.member) {
            problems.push(format!("the `[entrance]` member `{}` is not a declared `[software]` member", e.member));
        }
        if Path::new(&e.document).is_absolute() || Path::new(&e.document).components().any(|c| c == std::path::Component::ParentDir) {
            problems.push(format!("the `[entrance]` document `{}` must stay within the member's worktree (no `..` or absolute path)", e.document));
        }
    }

    let entrance_member = entrance.as_ref().map(|e| e.member.clone());
    let components = members.into_iter().filter(|m| entrance_member.as_deref() != Some(m.as_str())).collect();
    ProjectFacts { components, drivers, entrance, problems }
}

/// The kind named in a line's `<!-- host-reconcile: KIND -->` annotation, if any.
fn reconcile_kind(line: &str) -> Option<String> {
    let rest = &line[line.find(RECONCILE_MARK)? + RECONCILE_MARK.len()..];
    let kind = rest[..rest.find("-->")?].trim();
    (!kind.is_empty()).then(|| kind.to_string())
}

/// The visible text of an annotated line — the source line with the trailing
/// annotation comment removed, so the marker's own tokens never pollute the assertion.
fn reconcile_visible(line: &str) -> &str {
    match line.find(RECONCILE_MARK) {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Check one annotated restatement against the spine truth and the fixed layout.
/// Returns a HAZARD message, or None when the restatement still matches.
fn reconcile_assertion(kind: &str, visible: &str, facts: &ProjectFacts) -> Option<String> {
    let low = visible.to_ascii_lowercase();
    match kind {
        // The named set must still cover the declared components / rung-drivers; report
        // the omissions (a missing member is the drift, e.g. a README that drops
        // host-prove). An empty datum means a pre-data template — unverifiable, skip.
        "components" if !facts.components.is_empty() => {
            let missing: Vec<&str> = facts.components.iter().map(String::as_str).filter(|t| !visible.contains(*t)).collect();
            (!missing.is_empty()).then(|| format!("components restatement omits {} (declared host-* components: {})", missing.join(", "), facts.components.join(", ")))
        }
        "verification" if !facts.drivers.is_empty() => {
            let missing: Vec<&str> = facts.drivers.iter().map(String::as_str).filter(|d| !low.contains(&d.to_ascii_lowercase())).collect();
            (!missing.is_empty()).then(|| format!("verification-model restatement omits rung-driver {} (declared drivers: {})", missing.join(", "), facts.drivers.join(", ")))
        }
        "components" | "verification" => None,
        "where-root" => (!visible.contains("software/"))
            .then(|| "Where-root restatement does not name `software/` (the recorded Where layout)".to_string()),
        "spec-path" => (low.contains("plan/") && low.contains("spec"))
            .then(|| "spec-path restatement places specs under `plan/` — specs co-locate with their software (plan/0012)".to_string()),
        other => Some(format!("unknown host-reconcile kind `{other}` (known: {})", RECONCILE_KINDS.join(", "))),
    }
}

/// The concept vocabulary (plan/0039): the methodology concepts a project defines once
/// (a home heading carrying `{#id}`) and points at everywhere else (`[text](FILE#id)`)
/// instead of restating. The vocabulary is spine-universal (here); the values are
/// project-local (`.host-software`). `verifiers`/`software-root`/`spec-home` read the
/// former inline kinds `verification`/`where-root`/`spec-path` in plain words.
const CONCEPT_IDS: [&str; 4] = ["components", "verifiers", "software-root", "spec-home"];

/// Gather tracked markdown as `(relative-path, content)`: `git ls-files`, honoring
/// `.host-lintignore`, skipping symlinks and non-markdown. The one walk the reconcile
/// checks share.
fn tracked_markdown(root: &Path) -> Result<Vec<(String, String)>, String> {
    let ignore = load_lintignore(root);
    let out = process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err("git ls-files failed (reconcile needs a git repository)".to_string());
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    let mut docs = Vec::new();
    for rel in listing.split('\0').filter(|s| !s.is_empty()) {
        if !rel.to_ascii_lowercase().ends_with(".md") || path_ignored(rel, &ignore) {
            continue;
        }
        let path = root.join(rel);
        if fs::symlink_metadata(&path).map(|m| m.file_type().is_symlink()).unwrap_or(false) || !path.is_file() {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            docs.push((rel.to_string(), content));
        }
    }
    Ok(docs)
}

/// Collapse `.`/`..` in a relative path lexically, so two spellings of one target
/// compare equal. A `..` that would climb above the start is dropped.
fn normalize_rel(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when the relative `link_file`, written inside `doc_rel` and resolved against the
/// doc's directory, names `home_file` (a path from the repo root). Case-sensitive, as
/// mdBook's output is: a lowercased `structure.md` for `STRUCTURE.md` fails here, and
/// link-integrity surfaces the exact fix.
fn link_resolves_to(doc_rel: &str, link_file: &str, home_file: &str) -> bool {
    let doc_dir = Path::new(doc_rel).parent().unwrap_or_else(|| Path::new(""));
    normalize_rel(&doc_dir.join(link_file)) == normalize_rel(Path::new(home_file))
}

/// The ATX heading level of `line` (1..=6), or 0 if it is not a heading. mdBook honors
/// a `{#id}` attribute only on such a heading (`#` to `######` followed by a space), at
/// its end.
fn heading_level(line: &str) -> usize {
    let t = line.trim_start();
    let hashes = t.len() - t.trim_start_matches('#').len();
    if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
        hashes
    } else {
        0
    }
}

/// Whether `line` is an ATX heading.
fn is_heading(line: &str) -> bool {
    heading_level(line) != 0
}

/// True when byte offset `pos` falls outside an inline-`code`-span: an even number of
/// backticks precede it. The one inline-code-span guard the markdown scanners share (issue
/// #23), replacing the hand-copied `matches('`').count() % 2` idiom.
fn outside_code_span(s: &str, pos: usize) -> bool {
    s[..pos].matches('`').count().is_multiple_of(2)
}

/// The concept homes: `id -> [(file, line)]` for every **heading ending in `{#id}`** with
/// `id` in the vocabulary. mdBook honors `{#id}` only as a heading attribute at the *end*
/// of the heading: a `{#id}` on a non-heading line, or one at the *start* of a heading
/// (`## {#id} Title`), renders a different id or none, so a pointer to it would 404. One
/// inside an inline-code span is the syntax quoted, not a home.
fn scan_concept_anchors(docs: &[(String, String)]) -> std::collections::BTreeMap<String, Vec<(String, usize)>> {
    let mut anchors: std::collections::BTreeMap<String, Vec<(String, usize)>> = std::collections::BTreeMap::new();
    for (rel, content) in docs {
        // Mask fenced code (issue #12): a heading carrying `{#id}` inside a fenced example is
        // the syntax quoted, not a live concept home, so it must not register an anchor.
        let masked = mask_fenced_lines(content);
        for (n, line) in masked.iter().enumerate() {
            if !is_heading(line) {
                continue;
            }
            for id in CONCEPT_IDS {
                let needle = format!("{{#{id}}}");
                if line.trim_end().ends_with(&needle) {
                    let pos = line.find(&needle).unwrap_or(0);
                    if outside_code_span(line, pos) {
                        anchors.entry(id.to_string()).or_default().push((rel.clone(), n + 1));
                    }
                }
            }
        }
    }
    anchors
}

/// The concept links on a line: each markdown `[text](target)` whose `target` ends in
/// `#<id>` for a concept id. Returns `(file-part, id)`, file-part empty for a same-file
/// `#id`. A `](` inside an inline-code span is the documented syntax, not a live link.
fn concept_links_on(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = line[i..].find("](") {
        let open = i + rel;
        if !outside_code_span(line, open) {
            i = open + 2;
            continue;
        }
        let start = open + 2;
        let Some(close) = line[start..].find(')') else { break };
        let target = &line[start..start + close];
        if let Some((file, frag)) = target.rsplit_once('#') {
            if CONCEPT_IDS.contains(&frag) {
                out.push((file.to_string(), frag.to_string()));
            }
        }
        i = start + close + 1;
    }
    out
}

/// The text of a concept's home section in `content`: the heading line carrying the
/// anchor (1-based `home_line`) through the line before the next heading of the **same or
/// higher** level. A concept defined with deeper sub-headings (`## Components` then a
/// `### each-tool`) keeps every member inside its section, so coverage counts them all.
fn home_section(content: &str, home_line: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start = home_line.saturating_sub(1);
    let home_level = lines.get(start).map(|l| heading_level(l)).unwrap_or(0);
    let mut end = lines.len();
    for (k, l) in lines.iter().enumerate().skip(start + 1) {
        let lvl = heading_level(l);
        if lvl != 0 && lvl <= home_level {
            end = k;
            break;
        }
    }
    lines.get(start..end).map(|s| s.join("\n")).unwrap_or_default()
}

/// Whether `haystack` names `token` as a whole word — the token is delimited by a
/// non-identifier character (or a string boundary) on each side, so `host-reference` does not
/// match inside `host-reference-testkit` (issue #19: a recall-biased substring check misses
/// this). Identifier characters are ASCII alphanumerics plus `-` and `_`, the shape of a
/// component or driver name.
fn names_token(haystack: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(token) {
        let start = from + rel;
        let end = start + token.len();
        let before_ok = haystack[..start].chars().next_back().map(is_id) != Some(true);
        let after_ok = haystack[end..].chars().next().map(is_id) != Some(true);
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The concept-as-URI checks (plan/0039). **declared-anchor**: every concept link points
/// at a real concept that has a home. **link-integrity**: the link names the file holding
/// that home (case-sensitive). **coverage**: each project-local list concept's home names
/// every `.host-software` member — the bite the inline annotation carried. Plus a one-home
/// rule (a concept defined twice is ambiguous). One HAZARD per problem.
fn concept_checks(docs: &[(String, String)], facts: &ProjectFacts) -> Vec<String> {
    let mut hazards = Vec::new();
    let anchors = scan_concept_anchors(docs);
    for (id, homes) in &anchors {
        if homes.len() > 1 {
            let at: Vec<String> = homes.iter().map(|(f, l)| format!("{f}:{l}")).collect();
            hazards.push(format!("concept `{id}` is defined in more than one place ({}) — define it once and point at it", at.join(", ")));
        }
    }
    for (rel, content) in docs {
        // Mask fenced code (issue #12): a `](FILE#id)` inside a fenced example is documentation
        // of the link syntax, not a live link, so it must not be checked for resolution.
        let masked = mask_fenced_lines(content);
        for (n, line) in masked.iter().enumerate() {
            for (file, id) in concept_links_on(line) {
                match anchors.get(&id) {
                    None => hazards.push(format!("{rel}:{}: link to `#{id}` but no doc defines that concept (`{{#{id}}}`)", n + 1)),
                    Some(homes) => {
                        let ok = homes.iter().any(|(home_file, _)| {
                            if file.is_empty() {
                                home_file == rel
                            } else {
                                link_resolves_to(rel, &file, home_file)
                            }
                        });
                        if !ok {
                            let at: Vec<&str> = homes.iter().map(|(f, _)| f.as_str()).collect();
                            hazards.push(format!("{rel}:{}: concept link `{file}#{id}` does not resolve to the `{id}` home ({})", n + 1, at.join(", ")));
                        }
                    }
                }
            }
        }
    }
    // coverage — the bite: a project-local list concept's home names every member.
    let by_rel: std::collections::HashMap<&str, &str> = docs.iter().map(|(r, c)| (r.as_str(), c.as_str())).collect();
    let mut cover = |id: &str, members: &[String]| {
        if members.is_empty() {
            return;
        }
        let Some((file, line)) = anchors.get(id).and_then(|h| h.first()) else {
            return;
        };
        let Some(content) = by_rel.get(file.as_str()) else {
            return;
        };
        let section = home_section(content, *line);
        // Match on word boundaries (issue #19): a substring check would count `host-reference`
        // as covered by the longer `host-reference-testkit` and miss a genuinely-omitted member.
        let missing: Vec<&str> = members.iter().map(String::as_str).filter(|m| !names_token(&section, m)).collect();
        if !missing.is_empty() {
            hazards.push(format!("{file}:{line}: the `{id}` home omits {} (the project's {id}: {})", missing.join(", "), members.join(", ")));
        }
    };
    cover("components", &facts.components);
    cover("verifiers", &facts.drivers);
    // Single-value homes get a content bite too (issue #15): link-integrity alone would let the
    // software-root or spec-home home drift from the canonical fact while the pointers still
    // resolve. Each assertion is a positive affirmation correct against the real wording — NOT
    // the old inline `where-root`/`spec-path` predicate, whose `plan/`-and-`spec` form would
    // false-positive on the canonical spec-home text "co-located ... never under `plan/`".
    for (id, needle, msg) in [
        ("software-root", "software/", "the software-root home does not name `software/` (the recorded Where layout)"),
        ("spec-home", "co-locat", "the spec-home home does not affirm specs co-locate with their software (never under plan/)"),
    ] {
        let Some((file, line)) = anchors.get(id).and_then(|h| h.first()) else {
            continue;
        };
        let Some(content) = by_rel.get(file.as_str()) else {
            continue;
        };
        if !home_section(content, *line).to_ascii_lowercase().contains(needle) {
            hazards.push(format!("{file}:{line}: {msg}"));
        }
    }
    hazards
}

/// Count lines carrying a live (not code-spanned) inline `<!-- host-reconcile -->`
/// annotation — the deprecated form (plan/0039), surfaced as a migration note.
fn count_inline_annotations(docs: &[(String, String)]) -> usize {
    docs.iter()
        .flat_map(|(_, content)| content.lines())
        .filter(|line| match line.find(RECONCILE_MARK) {
            Some(pos) => outside_code_span(line, pos),
            None => false,
        })
        .count()
}

/// The inline-annotation scan (plan/0036, deprecated by plan/0039): check every live
/// `host-reconcile` annotation against the project's facts. Kept checking during the
/// transition so its bite holds until the form is retired. One HAZARD per failing
/// restatement (with `file:line`).
fn reconcile_scan(docs: &[(String, String)], facts: &ProjectFacts) -> Vec<String> {
    let mut hazards = Vec::new();
    for (rel, content) in docs {
        for (n, line) in content.lines().enumerate() {
            let Some(mpos) = line.find(RECONCILE_MARK) else { continue };
            // A marker inside an inline-code span is documentation of the syntax, not a
            // live directive (the spine and adopters quote it as an example). An odd
            // backtick count before the marker means it opens inside a code span.
            if !outside_code_span(line, mpos) {
                continue;
            }
            let Some(kind) = reconcile_kind(line) else { continue };
            if let Some(problem) = reconcile_assertion(&kind, reconcile_visible(line), facts) {
                hazards.push(format!("{rel}:{}: {problem}", n + 1));
            }
        }
    }
    hazards
}

/// `reconcile <dir>` — the reconcile arm. After a spine move stales a project's own
/// restatement of methodology, re-check each annotated restatement against the project's
/// own truth (its components and verifier drivers in `.host-software`) and the
/// fixed layout, and HAZARD a restatement that drifted. A `Flag` exits 1, so chaining
/// it into the verify recheck re-opens the gate when a restatement regresses.
fn reconcile(args: &[String]) {
    let dir = args.iter().find(|a| !a.starts_with("--")).map(String::as_str).unwrap_or(".");
    let root = match fs::canonicalize(Path::new(dir)) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    let facts = match fs::read_to_string(root.join(SOFTWARE)) {
        Ok(text) => parse_project_facts(&text),
        Err(_) => ProjectFacts::default(),
    };
    // A malformed `[entrance]` stanza corrupts the `components` set (a member wrongly kept in
    // or dropped), so surface it here, not only in `entrance` — else reconcile would demand
    // the front door be named a component, the call/0027 silent demotion (plan/0043 review).
    if !facts.problems.is_empty() {
        for p in &facts.problems {
            eprintln!("host-lifecycle: reconcile: {p}");
        }
        process::exit(2);
    }
    let docs = match tracked_markdown(&root) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("host-lifecycle: reconcile: {e}");
            process::exit(2);
        }
    };
    let mut hazards = concept_checks(&docs, &facts);
    // The inline annotation form is deprecated (plan/0039) but still checked during the
    // transition, so its bite holds until it is retired; warn on any survivor.
    hazards.extend(reconcile_scan(&docs, &facts));
    hazards.extend(plan_index_problems(&docs));
    let inline = count_inline_annotations(&docs);
    if inline > 0 {
        eprintln!("host-lifecycle: reconcile: note — {inline} inline `<!-- host-reconcile -->` annotation(s) are deprecated (plan/0039); migrate each restatement to a concept pointer `[text](STRUCTURE.md#id)`");
    }
    if hazards.is_empty() {
        println!("reconcile: clean (every concept link resolves to its home; each home covers its .host-software set)");
        return;
    }
    for h in &hazards {
        println!("HAZARD   {h}");
    }
    eprintln!("host-lifecycle: reconcile — {} concept drift(s)", hazards.len());
    process::exit(1);
}

// --- plan/0040: the entrance check ---
//
// The single-file entrance (the `.host-software` member marked `entrance = true`) is a
// published README in its own repo, outside any host's verify gate, so its restatements of
// the spine stale silently (plan/0040). It must stay self-contained, so it cannot point at
// the spine with a link the way an in-host doc does (the reconcile arm); it restates. This
// holds the restatement to the spine's structured facts: coverage of the lifecycle phases
// (the manifest) and the wired tools (the `.host-software` verifier drivers plus the
// lifecycle engine), and byte-exact generation of the `.host` stamp block. The version pins,
// the lanes rule, and the tool prose have no structured home and stay authored (the design
// review settled this; a structured pin home is a named follow-up).

/// The canonical `.host` stamp block the entrance shows, with placeholder values — the
/// same format `adopt` writes (`stamp_body`), so the entrance cannot restate it wrong.
fn entrance_stamp() -> String {
    stamp_body("<sha-or-tag>", "YYYY-MM-DD")
}

/// The body of the first fenced code block after the `## The stamp` heading (the `.host`
/// stamp block the entrance shows), or None when the heading or a following fence is
/// absent (a new `## ` section before a fence ends the search).
fn entrance_stamp_block(content: &str) -> Option<String> {
    let mut lines = content.lines();
    for line in lines.by_ref() {
        if line.trim_start().starts_with("## The stamp") {
            break;
        }
    }
    let mut in_block = false;
    let mut block = String::new();
    for line in lines {
        let t = line.trim_start();
        if !in_block {
            if t.starts_with("```") {
                in_block = true;
            } else if t.starts_with("## ") {
                return None;
            }
            continue;
        }
        if t.starts_with("```") {
            return Some(block);
        }
        block.push_str(line);
        block.push('\n');
    }
    None
}

/// The entrance coverage and stamp problems, for the concepts the entrance declares it
/// `restates`. A phase is checked as a backtick token (`` `release` ``), since a bare word
/// like "release" recurs in prose ("GitHub releases"); a tool name is distinctive, so a plain
/// mention suffices; the stamp block is checked against the canonical format. A concept the
/// entrance does not restate is not checked (plan/0043).
///
/// The tool check is deliberately a lenient whole-document presence test (issue #20 disposition,
/// plan/0051): the entrance is a human-facing README whose host-* tool names are distinctive and
/// hyphenated, so a plain mention anywhere is a faithful restatement, and a word-boundary or
/// prose-only rule would flag legitimate phrasings (a tool named only inside a `[text](url)`
/// link, or a heading) while still not catching a token buried in a bare href. The real bite is
/// elsewhere: the phase backtick tokens and the byte-exact stamp block. Left lenient by design.
fn entrance_problems(content: &str, phases: &[String], tools: &[String], restates: &Restates) -> Vec<String> {
    let low = content.to_ascii_lowercase();
    let mut problems = Vec::new();
    if restates.checks("phases") {
        for p in phases {
            if !content.contains(&format!("`{p}`")) {
                problems.push(format!("the entrance omits the `{p}` lifecycle phase (the manifest declares it)"));
            }
        }
    }
    if restates.checks("tools") {
        // Lenient whole-document presence by design — see the function doc (issue #20).
        for t in tools {
            if !low.contains(&t.to_ascii_lowercase()) {
                problems.push(format!("the entrance omits the wired tool `{t}` (declared in {SOFTWARE})"));
            }
        }
    }
    if restates.checks("stamp") {
        match entrance_stamp_block(content) {
            None => problems.push("the entrance has no `.host` stamp code block under `## The stamp`".into()),
            Some(block) if block.trim() != entrance_stamp().trim() => {
                problems.push("the `.host` stamp block drifted from the canonical format; regenerate with `entrance`".into());
            }
            Some(_) => {}
        }
    }
    problems
}

/// Rewrite the `.host` stamp block under `## The stamp` to the canonical format, returning
/// the updated document, or None when the heading or its fence is absent.
fn entrance_regenerate_stamp(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let h = lines.iter().position(|l| l.trim_start().starts_with("## The stamp"))?;
    let open = (h + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with("```"))?;
    if (h + 1..open).any(|i| lines[i].trim_start().starts_with("## ")) {
        return None;
    }
    let close = (open + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with("```"))?;
    let mut out: Vec<String> = lines[..=open].iter().map(|s| s.to_string()).collect();
    out.extend(entrance_stamp().lines().map(str::to_string));
    out.extend(lines[close..].iter().map(|s| s.to_string()));
    let mut s = out.join("\n");
    if content.ends_with('\n') {
        s.push('\n');
    }
    Some(s)
}

/// The lifecycle phase names (the manifest) and the wired tool names (the `.host-software`
/// verifier drivers plus the lifecycle engine `host-lifecycle`) the entrance must name.
fn entrance_spine_facts(root: &Path, facts: &ProjectFacts) -> Result<(Vec<String>, Vec<String>), String> {
    let phases = match load_project_manifest(root) {
        ManifestState::Live(ps) => ps.iter().map(|p| p.name.clone()).collect(),
        _ => return Err(format!("the lifecycle manifest is unavailable (a {STAMP} stamp and host-template are needed)")),
    };
    let mut tools = facts.drivers.clone();
    if !tools.iter().any(|t| t == "host-lifecycle") {
        tools.push("host-lifecycle".to_string());
    }
    Ok((phases, tools))
}

/// Resolve the entrance document and its content from the `[entrance]` stanza: the member's
/// worktree joined with `document` (default `README.md`), so a `SKILL.md` or a landing page
/// is reached by path (plan/0043).
fn entrance_readme(root: &Path, facts: &ProjectFacts) -> Result<(PathBuf, String), String> {
    let e = facts.entrance.as_ref().ok_or_else(|| format!("no `[entrance]` stanza in {SOFTWARE} (declare one naming the entrance member)"))?;
    let branch = load_software(root).into_iter().find(|s| s.name == e.member).map(|s| s.branch).unwrap_or_else(|| "main".to_string());
    let path = worktree_dir(root, &e.member, &branch).join(&e.document);
    let content = fs::read_to_string(&path).map_err(|err| format!("cannot read the entrance {}: {err} (materialize the `{}` worktree)", path.display(), e.member))?;
    Ok((path, content))
}

/// `entrance [--check] [<dir>]`: hold the single-file entrance to the spine's structured
/// facts (plan/0040). Without `--check`, regenerate the `.host` stamp block in place and
/// report any coverage gap; with `--check`, exit non-zero on any gap or stamp drift.
fn entrance(args: &[String]) {
    let mut check = false;
    let mut dir = String::from(".");
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            s if s.starts_with("--") => {}
            s => dir = s.to_string(),
        }
    }
    let root = Path::new(&dir);
    let raw = match fs::read_to_string(root.join(SOFTWARE)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("host-lifecycle: entrance needs {SOFTWARE}: {e}");
            process::exit(2);
        }
    };
    let facts = parse_project_facts(&raw);
    if !facts.problems.is_empty() {
        for p in &facts.problems {
            eprintln!("host-lifecycle: entrance: {p}");
        }
        process::exit(2);
    }
    let (readme, content) = match entrance_readme(root, &facts) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("host-lifecycle: {e}");
            process::exit(2);
        }
    };
    let (phases, tools) = match entrance_spine_facts(root, &facts) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("host-lifecycle: entrance: {e}");
            process::exit(2);
        }
    };
    let restates = facts.entrance.as_ref().map(|e| e.restates.clone()).unwrap_or(Restates::All);

    if check {
        let problems = entrance_problems(&content, &phases, &tools, &restates);
        if problems.is_empty() {
            println!("entrance: clean (every restated concept is complete in {})", readme.display());
            return;
        }
        for p in &problems {
            println!("HAZARD   {p}");
        }
        eprintln!("host-lifecycle: entrance — {} drift(s) in {}", problems.len(), readme.display());
        process::exit(1);
    }

    // Write mode: regenerate the one generated block (the stamp) when the entrance restates it;
    // a coverage gap is prose the author fills, so report it rather than rewrite.
    if restates.checks("stamp") {
        match entrance_regenerate_stamp(&content) {
            Some(updated) if updated != content => {
                if let Err(e) = fs::write(&readme, updated) {
                    eprintln!("host-lifecycle: cannot write {}: {e}", readme.display());
                    process::exit(2);
                }
                println!("entrance: regenerated the `.host` stamp block in {}", readme.display());
            }
            Some(_) => println!("entrance: the `.host` stamp block is already canonical"),
            None => {
                eprintln!("host-lifecycle: no `.host` stamp code block under `## The stamp` in {}", readme.display());
                process::exit(2);
            }
        }
    } else {
        println!("entrance: the entrance does not restate the stamp; nothing to regenerate in {}", readme.display());
    }
    for g in entrance_problems(&content, &phases, &tools, &restates).into_iter().filter(|p| !p.contains("stamp")) {
        println!("note: {g}");
    }
}

// --- plan/0042: the receipted task graph ---
//
// An in-plan task is an anchored `### ` heading under the `## Build sequence` section of a
// `plan/NNNN-slug/README.md`. Its global id (and ledger key) is `plan/NNNN#anchor`, so a
// receipt and a dependency edge hang on a stable anchor, not a position. `depends` names the
// prerequisites (a local `#anchor` or a cross-milestone `plan/NNNN#anchor`); the tool derives
// the parallel frontier, the author never does (call/0024).

#[derive(Clone, PartialEq, Debug)]
enum TaskVerify {
    /// A shell command the gate re-runs (mechanical).
    Mechanical(String),
    /// `call/NNNN` or `operator`: attested, discharged by the citation resolving.
    Attested(String),
}

#[derive(Clone)]
struct Task {
    key: String,          // "plan/0042#implement-parser" — global id and ledger key (plan#anchor)
    rel: String,          // "plan/0042-receipted-task-graph/README.md"
    line: usize,          // 1-based heading line
    depends: Vec<String>, // resolved global keys
    verify: Option<TaskVerify>,
    inputs: Vec<String>,
}

/// `plan/NNNN` from a tracked path `plan/NNNN-slug/README.md`, else None. The number is the
/// milestone identity; the slug is content.
fn plan_id_of(rel: &str) -> Option<String> {
    let mut parts = rel.split('/');
    if parts.next()? != "plan" {
        return None;
    }
    let dir = parts.next()?;
    if parts.next()? != "README.md" || parts.next().is_some() {
        return None;
    }
    let num = dir.split('-').next()?;
    if num.len() == 4 && num.bytes().all(|b| b.is_ascii_digit()) {
        Some(format!("plan/{num}"))
    } else {
        None
    }
}

/// The milestone DIRECTORY of a `plan/NNNN-slug/README.md` rel path (`plan/NNNN-slug`),
/// the form a PLAN.md row links. `plan_id_of` yields the numeric id; this yields the
/// linkable directory, so plan-index coverage keys on the exact link target.
fn milestone_dir_of(rel: &str) -> Option<String> {
    let mut parts = rel.split('/');
    if parts.next()? != "plan" {
        return None;
    }
    let dir = parts.next()?;
    if parts.next()? != "README.md" || parts.next().is_some() {
        return None;
    }
    let num = dir.split('-').next()?;
    if num.len() == 4 && num.bytes().all(|b| b.is_ascii_digit()) {
        Some(format!("plan/{dir}"))
    } else {
        None
    }
}

/// Every `plan/NNNN-slug/README.md` link target inside a PLAN.md body.
fn plan_readme_link_targets(index: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in index.match_indices("](plan/") {
        let start = i + 2; // skip the "]("
        if let Some(close) = index[start..].find(')') {
            let target = &index[start..start + close];
            if target.starts_with("plan/") && target.ends_with("/README.md") {
                out.push(target.to_string());
            }
        }
    }
    out
}

/// plan/0062: PLAN.md must index every milestone directory. The audited-plan rule
/// (each milestone gets a PLAN.md row) is asserted in the spine but gated by nothing,
/// so plan directories drifted unindexed. This derives the owed set from repo state
/// each run — every tracked `plan/NNNN-slug/README.md` owes a linked row — and HAZARDs
/// any directory with no live row, plus the reverse (a row linking a directory that
/// does not exist). Fail-safe and self-re-listing like the component-coverage check,
/// keyed on the table-row LINK TARGET rather than a bare prose mention, since the
/// defect was directories discussed in prose yet absent from the table. A row link
/// inside a fenced block is masked out, so an example link never counts as live.
fn plan_index_problems(docs: &[(String, String)]) -> Vec<String> {
    let owed: Vec<String> = docs.iter().filter_map(|(rel, _)| milestone_dir_of(rel)).collect();
    let Some((_, plan)) = docs.iter().find(|(r, _)| r == "PLAN.md") else {
        return Vec::new();
    };
    let index = mask_fenced_lines(plan).join("\n");
    let mut hz = Vec::new();
    for dir in &owed {
        if !index.contains(&format!("]({dir}/README.md)")) {
            hz.push(format!(
                "PLAN.md: the milestone index omits {dir} (a plan/ directory with no linked row)"
            ));
        }
    }
    for target in plan_readme_link_targets(&index) {
        let dir = target.trim_end_matches("/README.md");
        if !owed.iter().any(|d| d == dir) {
            hz.push(format!(
                "PLAN.md: a milestone row links {target} but no such plan/ directory exists"
            ));
        }
    }
    hz
}

/// The slug `{#anchor}` at the END of a heading line (the placement mdBook honors), if any.
/// The anchor is a `[a-z0-9-]+` slug. A `{#...}` inside an inline-code span is skipped (it is
/// the syntax quoted, not a live anchor).
fn heading_end_anchor(line: &str) -> Option<String> {
    let t = line.trim_end();
    let close = t.strip_suffix('}')?;
    let open = close.rfind("{#")?;
    if !outside_code_span(t, open) {
        return None;
    }
    let id = &close[open + 2..];
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        Some(id.to_string())
    } else {
        None
    }
}

/// Whether `line` is the `## Build sequence` section heading (the one section whose `### `
/// headings are tasks).
fn is_build_sequence_heading(line: &str) -> bool {
    if heading_level(line) != 2 {
        return false;
    }
    line.trim_start().trim_start_matches('#').trim().eq_ignore_ascii_case("Build sequence")
}

/// A heading title to its anchor slug: lowercase, every run of non-alphanumeric characters
/// to a single `-`, trimmed. `tasks --new` emits the exact `{#slug}` so the agent never
/// hand-types the anchor (the fill-in-the-blank fold-back for the weak-agent bar).
fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Resolve a `depends` value to global keys. A local `#anchor` becomes `plan/NNNN#anchor`; a
/// `plan/NNNN#anchor` passes through; `(none)` (or empty) is an explicit root.
fn parse_depends(v: &str, plan: &str) -> Vec<String> {
    let v = v.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("(none)") || v.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    v.split(',')
        .filter_map(|r| {
            let r = r.trim();
            if r.is_empty() {
                None
            } else if let Some(a) = r.strip_prefix('#') {
                Some(format!("{plan}#{a}"))
            } else {
                Some(r.to_string())
            }
        })
        .collect()
}

/// Parse a `verify` value: a shell command (mechanical), or `attested <call/NNNN | operator>`.
fn parse_verify(v: &str) -> Result<TaskVerify, String> {
    let v = v.trim();
    if let Some(att) = v.strip_prefix("attested ") {
        let att = att.trim();
        if att == "operator" || (att.starts_with("call/") && att.len() > "call/".len()) {
            Ok(TaskVerify::Attested(att.to_string()))
        } else {
            Err(format!("`verify: attested {att}` must cite `operator` or `call/NNNN`"))
        }
    } else if v.is_empty() {
        Err("`verify:` needs a command, or `attested <call/NNNN|operator>`".to_string())
    } else {
        Ok(TaskVerify::Mechanical(v.to_string()))
    }
}

/// Parse the tasks from the tracked plan READMEs, plus the structural HAZARDs: an anchored
/// `### ` outside `## Build sequence` (a task anchor must be a task), a build-sequence `### `
/// with no end-anchor, or a malformed `verify`. A task's `depends`/`verify`/`inputs` are the
/// `- key: value` bullets that follow its heading.
/// Blank the lines inside fenced code blocks (keeping the line count so reported line numbers
/// stay accurate). A `### ` task or a `## Build sequence` inside a ``` example is then not read
/// as a heading.
fn mask_fenced_lines(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in content.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push(String::new());
        } else if in_fence {
            out.push(String::new());
        } else {
            out.push(line.to_string());
        }
    }
    out
}

fn parse_tasks(docs: &[(String, String)]) -> (Vec<Task>, Vec<String>) {
    let mut tasks = Vec::new();
    let mut problems = Vec::new();
    for (rel, content) in docs {
        let Some(plan) = plan_id_of(rel) else { continue };
        let masked = mask_fenced_lines(content);
        let lines: Vec<&str> = masked.iter().map(|s| s.as_str()).collect();
        let mut in_build_seq = false;
        let mut prev_in_plan: Option<String> = None;
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let lvl = heading_level(line);
            if lvl == 2 {
                in_build_seq = is_build_sequence_heading(line);
                i += 1;
                continue;
            }
            if lvl != 3 {
                i += 1;
                continue;
            }
            let anchor = heading_end_anchor(line);
            if !in_build_seq {
                if anchor.is_some() {
                    problems.push(format!("{rel}:{}: an anchored `### ` heading belongs under `## Build sequence` (a task anchor must be a task)", i + 1));
                }
                i += 1;
                continue;
            }
            // Scan this heading's bullets once. A standalone `- band` marker makes it a
            // BAND: a content-named grouping over the anchored tasks that follow it
            // (host-lifecycle#4, plan/0066). The marker attaches only to its own
            // immediately-preceding heading, so a bare `- band` line, not a look-ahead,
            // is the signal (the qwen3.5-4b scoring flagged look-ahead as the mis-read).
            let mut band = false;
            let mut depends: Option<Vec<String>> = None;
            let mut verify: Option<TaskVerify> = None;
            let mut inputs: Vec<String> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() && heading_level(lines[j]) == 0 {
                let b = lines[j].trim();
                if b == "- band" {
                    band = true;
                } else if let Some(v) = b.strip_prefix("- depends:") {
                    depends = Some(parse_depends(v, &plan));
                } else if let Some(v) = b.strip_prefix("- verify:") {
                    match parse_verify(v) {
                        Ok(tv) => verify = Some(tv),
                        Err(e) => problems.push(format!("{rel}:{}: {e}", j + 1)),
                    }
                } else if let Some(v) = b.strip_prefix("- inputs:") {
                    inputs = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                }
                j += 1;
            }
            // A band groups but is not a task: it pushes no node, carries no receipt or
            // edge, and does NOT reset the linear default, so task-to-task chaining
            // continues across the band heading. Order lives in the depends edges, never
            // in the band's name or position.
            if band {
                i = j;
                continue;
            }
            let Some(anchor) = anchor else {
                problems.push(format!("{rel}:{}: a `## Build sequence` task heading needs an anchor at its end (`### Title {{#anchor}}`), or a `- band` marker to make it a group", i + 1));
                i = j;
                continue;
            };
            let key = format!("{plan}#{anchor}");
            // Linear default: no explicit `depends` means the previous task in this plan's
            // build sequence (the first task is a root).
            let depends = depends.unwrap_or_else(|| prev_in_plan.iter().cloned().collect());
            tasks.push(Task { key: key.clone(), rel: rel.clone(), line: i + 1, depends, verify, inputs });
            prev_in_plan = Some(key);
            i = j;
        }
    }
    (tasks, problems)
}

/// Graph HAZARDs over the resolved tasks: a `depends` naming a non-task (dangling), and a
/// dependency cycle (which spans milestones, since an anchor is a global key). Topological
/// removal: a task is removable once its real-task deps are gone; a residue is a cycle.
fn task_graph_problems(tasks: &[Task]) -> Vec<String> {
    let mut problems = Vec::new();
    let keys: std::collections::HashSet<String> = tasks.iter().map(|t| t.key.clone()).collect();
    for t in tasks {
        for d in &t.depends {
            if !keys.contains(d) {
                problems.push(format!("{}:{}: task `{}` depends on `{}`, which is not a task (dangling dependency)", t.rel, t.line, t.key, d));
            }
        }
    }
    let mut remaining: std::collections::HashMap<String, Vec<String>> = tasks
        .iter()
        .map(|t| (t.key.clone(), t.depends.iter().filter(|d| keys.contains(*d)).cloned().collect()))
        .collect();
    loop {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|(_, deps)| deps.iter().all(|d| !remaining.contains_key(d)))
            .map(|(k, _)| k.clone())
            .collect();
        if ready.is_empty() {
            break;
        }
        for k in &ready {
            remaining.remove(k);
        }
    }
    if !remaining.is_empty() {
        let mut involved: Vec<String> = remaining.into_keys().collect();
        involved.sort();
        problems.push(format!("task dependency cycle among: {}", involved.join(", ")));
    }
    problems
}

/// The `plan/NNNN#anchor` task-reference tokens on a line, outside inline-code spans. The
/// short key form is unambiguously a task reference (a clickable doc link uses the full path).
fn plan_anchor_tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while let Some(rel) = line[i..].find("plan/") {
        let start = i + rel;
        // code-span guard: a token inside an inline-code span is quoted, not a live reference.
        if !outside_code_span(line, start) {
            i = start + 5;
            continue;
        }
        let mut j = start + 5;
        let num_start = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j - num_start == 4 && j < bytes.len() && bytes[j] == b'#' {
            let anchor_start = j + 1;
            let mut k = anchor_start;
            while k < bytes.len() && (bytes[k].is_ascii_lowercase() || bytes[k].is_ascii_digit() || bytes[k] == b'-') {
                k += 1;
            }
            if k > anchor_start {
                out.push(line[start..k].to_string());
            }
            i = k;
        } else {
            i = start + 5;
        }
    }
    out
}

/// Reference-integrity: every `plan/NNNN#anchor` task reference in prose resolves to a real
/// task. The `depends` bullets are skipped here (the graph check covers their resolution),
/// so a dangling dep is reported once.
fn task_reference_problems(docs: &[(String, String)], keys: &std::collections::HashSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    for (rel, content) in docs {
        let mut in_fence = false;
        for (n, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            // A fenced code block holds illustrative references (the grammar examples, a ledger
            // stanza), not live ones; toggle on its fence and skip its body.
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence || trimmed.starts_with("- depends:") {
                continue;
            }
            for tok in plan_anchor_tokens(line) {
                if !keys.contains(&tok) {
                    problems.push(format!("{rel}:{}: task reference `{tok}` does not resolve to a task", n + 1));
                }
            }
        }
    }
    problems
}

/// The full structural sweep over tracked docs: parse problems, graph problems, and
/// reference problems. Used by `validate plan/` and the task gate.
fn task_structure_problems(docs: &[(String, String)]) -> Vec<String> {
    let (tasks, mut problems) = parse_tasks(docs);
    problems.extend(task_graph_problems(&tasks));
    let keys: std::collections::HashSet<String> = tasks.iter().map(|t| t.key.clone()).collect();
    problems.extend(task_reference_problems(docs, &keys));
    problems
}

const TASK_RECEIPTS: &str = ".host-task-receipts";

/// A receipt for one task, in `.host-task-receipts`: `[receipt "plan/NNNN#anchor"]` stanzas
/// (the git-config form the other receipt ledgers use; call/0024). A third receipt kind
/// beside the methodology-version (`.host-receipts`) and operational
/// (`.host-lifecycle-receipts`) ledgers, by the operator's ruling over the ontology objection.
struct TaskReceipt {
    key: String,
    disposition: String,
    verify: Option<String>,
    inputs: Option<String>,
    digest: Option<String>,
    evidence: Option<String>,
    reason: Option<String>,
    tool: Option<String>,
    recorded: Option<String>,
}

fn parse_task_receipts(text: &str) -> Vec<TaskReceipt> {
    let mut out: Vec<TaskReceipt> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(inner) = t.strip_prefix("[receipt \"").and_then(|r| r.strip_suffix("\"]")) {
            out.push(TaskReceipt {
                key: inner.to_string(),
                disposition: String::new(),
                verify: None,
                inputs: None,
                digest: None,
                evidence: None,
                reason: None,
                tool: None,
                recorded: None,
            });
            continue;
        }
        let Some((k, v)) = t.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let Some(cur) = out.last_mut() else { continue };
        match k {
            "disposition" => cur.disposition = v.to_string(),
            "verify" => cur.verify = Some(v.to_string()),
            "inputs" => cur.inputs = Some(v.to_string()),
            "digest" => cur.digest = Some(v.to_string()),
            "evidence" => cur.evidence = Some(v.to_string()),
            "reason" => cur.reason = Some(v.to_string()),
            "tool" => cur.tool = Some(v.to_string()),
            "recorded" => cur.recorded = Some(v.to_string()),
            _ => {}
        }
    }
    out
}

/// The current receipt for a task key: the LAST matching stanza (append-only, last-wins).
fn latest_task_receipt<'a>(receipts: &'a [TaskReceipt], key: &str) -> Option<&'a TaskReceipt> {
    receipts.iter().rev().find(|r| r.key == key)
}

fn task_receipt_stanza(r: &TaskReceipt) -> String {
    let mut s = format!("[receipt \"{}\"]\n    disposition = {}\n", r.key, r.disposition);
    for (k, v) in [
        ("verify", &r.verify),
        ("inputs", &r.inputs),
        ("digest", &r.digest),
        ("evidence", &r.evidence),
        ("reason", &r.reason),
        ("tool", &r.tool),
        ("recorded", &r.recorded),
    ] {
        if let Some(v) = v {
            s.push_str(&format!("    {k} = {v}\n"));
        }
    }
    s
}

fn read_task_receipts(root: &Path) -> Vec<TaskReceipt> {
    parse_task_receipts(&fs::read_to_string(root.join(TASK_RECEIPTS)).unwrap_or_default())
}

fn append_task_receipt(root: &Path, r: &TaskReceipt) -> std::io::Result<()> {
    let path = root.join(TASK_RECEIPTS);
    let mut cur = fs::read_to_string(&path).unwrap_or_default();
    if !cur.is_empty() {
        if !cur.ends_with('\n') {
            cur.push('\n');
        }
        cur.push('\n');
    }
    cur.push_str(&task_receipt_stanza(r));
    write_atomic(&path, &cur)
}

/// The verdict for one task against its current receipt (the cheap path, the obligations
/// staleness model rather than command execution): an attested `done` resolves its citation,
/// a mechanical `done` is fresh by input-digest, a verify that changed since the receipt is
/// stale, and a `skip` carries a resolvable reason. `(ok, note)`; pure but for fs reads, so
/// it is fixture-testable. The full re-run of a mechanical verify is `tasks --rederive`.
fn task_verdict(root: &Path, t: &Task, r: &TaskReceipt) -> (bool, String) {
    match r.disposition.as_str() {
        "done" => {
            let current = match &t.verify {
                None => return (false, "done but the task declares no `verify` — a done must be re-derivable".into()),
                Some(TaskVerify::Mechanical(c)) => c.clone(),
                Some(TaskVerify::Attested(c)) => format!("attested {c}"),
            };
            if r.verify.as_deref() != Some(current.as_str()) {
                return (false, "the task's `verify` changed since the receipt was recorded; re-derive".into());
            }
            match &t.verify {
                Some(TaskVerify::Attested(c)) => {
                    if c == "operator" {
                        (true, "done (attested: operator)".into())
                    } else if cited_decision_exists(root, c) {
                        (true, format!("done (attested: {c})"))
                    } else {
                        (false, format!("done attested by `{c}`, which does not resolve to a `call/` decision"))
                    }
                }
                Some(TaskVerify::Mechanical(_)) => {
                    if t.inputs.is_empty() {
                        (true, "done (mechanical; declare `inputs` to track staleness)".into())
                    } else {
                        match &r.digest {
                            None => (true, "done (mechanical; run --record-digests to track staleness)".into()),
                            Some(want) => match input_digest(&t.inputs, root) {
                                Ok(got) if &got == want => (true, "done (mechanical; inputs fresh)".into()),
                                Ok(_) => (false, "STALE — the verify inputs changed since the recorded re-derivation; re-derive".into()),
                                Err(e) => (false, format!("STALE — {e}")),
                            },
                        }
                    }
                }
                None => unreachable!(),
            }
        }
        // A skip must cite a real decision, exactly as the recorder `tasks_record` requires
        // (issue #7): a free-text reason such as `wip` is an unaccountable justification, and
        // the gate is the fail-safe authority over hand-edited receipt files.
        "skip" => match &r.reason {
            None => (false, "skip without a `reason`".into()),
            Some(reason) => {
                if !valid_skip_citation(reason) {
                    (false, format!("skip cites `{reason}`, which is not a `call/NNNN` id — a skip must cite a decision"))
                } else if !cited_decision_exists(root, reason) {
                    (false, format!("skip cites `{reason}`, which does not resolve to a `call/` decision"))
                } else {
                    (true, format!("skip ({reason})"))
                }
            }
        },
        other => (false, format!("unknown disposition `{other}`")),
    }
}

/// The per-task gate: every declared task needs a receipt the verdict accepts, and a receipt
/// whose anchor no longer names a task is an orphan (a renamed or removed task left a stale
/// receipt, the reverse-drift check). One `GateLine` each.
/// The completion words a `## Status` first line may begin with (issue #18): the repo's
/// vocabulary, not only three of them, so a synonym is not read as an open milestone.
const COMPLETION_WORDS: [&str; 5] = ["complete", "done", "landed", "shipped", "released"];

/// Whether a milestone is marked complete, read from its README `## Status` section: the
/// first non-empty line begins with a completion word (the repo's Status convention, for
/// example "complete, landed …" or "done (…)"). An open milestone's tasks may be pending; a
/// complete one owes a done or skip receipt per task.
fn milestone_complete(content: &str) -> bool {
    // Read the masked view so a fenced `## Status` example cannot flip the verdict (issue #11);
    // the sibling task-gate parsers already mask fences, so the two no longer disagree.
    let masked = mask_fenced_lines(content);
    let mut in_status = false;
    for line in &masked {
        if heading_level(line) == 2 {
            in_status = line.trim_start().trim_start_matches('#').trim().eq_ignore_ascii_case("Status");
            continue;
        }
        if in_status {
            // Strip leading list markers (`-`, `*`, `+`) before the completion word, and
            // recognise the repo's completion synonyms, not only three of them (issue #18).
            let t = line.trim_start_matches(['-', '*', '+', ' ', '\t']).to_ascii_lowercase();
            if t.is_empty() {
                continue;
            }
            return COMPLETION_WORDS.iter().any(|w| t.starts_with(w));
        }
    }
    false
}

fn task_gate(root: &Path, tasks: &[Task], receipts: &[TaskReceipt], complete_plans: &std::collections::HashSet<String>) -> Vec<GateLine> {
    let mut out = Vec::new();
    let task_keys: std::collections::HashSet<&str> = tasks.iter().map(|t| t.key.as_str()).collect();
    for t in tasks {
        let label = format!("task {}", t.key);
        let plan = t.key.split('#').next().unwrap_or("");
        let line = match latest_task_receipt(receipts, &t.key) {
            // No receipt: a task in a milestone marked complete owes one; in an open milestone
            // it is simply pending future work, so it does not gate.
            None if complete_plans.contains(plan) => GateLine {
                label,
                ok: false,
                note: "no receipt (its milestone is marked complete)".into(),
                recheck: None,
                remedy: Some(format!("host-lifecycle tasks --record {} --disposition done --evidence <...>", t.key)),
            },
            None => GateLine {
                label,
                ok: true,
                note: "pending (no receipt; milestone open)".into(),
                recheck: None,
                remedy: None,
            },
            Some(r) => {
                let (ok, note) = task_verdict(root, t, r);
                GateLine { label, ok, note, recheck: None, remedy: None }
            }
        };
        out.push(line);
    }
    let mut seen = std::collections::HashSet::new();
    for r in receipts.iter().rev() {
        if !seen.insert(r.key.as_str()) {
            continue;
        }
        if !task_keys.contains(r.key.as_str()) {
            out.push(GateLine {
                label: format!("task {}", r.key),
                ok: false,
                note: "orphan receipt — no task carries this anchor (a renamed or removed task left a stale receipt)".into(),
                recheck: None,
                remedy: None,
            });
        }
    }
    out
}

/// A task's mechanical verify runs during `tasks --rederive`. Refuse one that re-enters the
/// gate (an infinite-recursion footgun), since `run_recheck` does not set the in-check guard.
fn verify_command_is_safe(cmd: &str) -> bool {
    !(cmd.contains("software --check") || cmd.contains("software --verify-build") || cmd.contains("tasks --rederive"))
}

/// The full task sweep over the tracked docs: structural problems (parse, graph, reference)
/// and the per-task receipt gate. Inert when no task is declared and no task receipt exists,
/// so it costs nothing until a plan adopts the form. Returns the HAZARD count.
fn task_check_problems(root: &Path) -> usize {
    let docs = match tracked_markdown(root) {
        Ok(d) => d,
        Err(_) => return 0, // not a git repo: inert
    };
    let mut bad = 0;
    for p in task_structure_problems(&docs) {
        println!("HAZARD   {p}");
        bad += 1;
    }
    let (tasks, _) = parse_tasks(&docs);
    let receipts = read_task_receipts(root);
    if tasks.is_empty() && receipts.is_empty() {
        return bad;
    }
    let complete_plans: std::collections::HashSet<String> = docs
        .iter()
        .filter_map(|(rel, content)| plan_id_of(rel).filter(|_| milestone_complete(content)))
        .collect();
    for line in task_gate(root, &tasks, &receipts, &complete_plans) {
        if line.ok {
            println!("ok       {} — {}", line.label, line.note);
        } else {
            println!("HAZARD   {} — {}", line.label, line.note);
            if let Some(rem) = &line.remedy {
                println!("           remedy: {rem}");
            }
            bad += 1;
        }
    }
    bad
}

/// `tasks <dir>` — the receipted task graph (plan/0042). Default prints the status (the
/// tasks, their receipts, and the ready frontier); `--check` runs the gate; `--record`
/// writes a receipt (pulling the task's own `verify`/`inputs`, so the agent never re-types
/// them); `--rederive` re-runs each mechanical `done` and refreshes its input digest.
fn tasks(args: &[String]) {
    let mut mode = "status";
    let (mut key, mut disposition, mut evidence, mut reason, mut title) = (None, None, None, None, None);
    let mut record_digests = false;
    let mut dir = String::from(".");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => {
                mode = "check";
                i += 1;
            }
            "--rederive" => {
                mode = "rederive";
                i += 1;
            }
            "--new" => {
                mode = "new";
                title = args.get(i + 1).cloned();
                i += 2;
            }
            "--record" => {
                mode = "record";
                key = args.get(i + 1).cloned();
                i += 2;
            }
            "--disposition" => {
                disposition = args.get(i + 1).cloned();
                i += 2;
            }
            "--evidence" => {
                evidence = args.get(i + 1).cloned();
                i += 2;
            }
            "--reason" => {
                reason = args.get(i + 1).cloned();
                i += 2;
            }
            "--record-digests" => {
                record_digests = true;
                i += 1;
            }
            s if s.starts_with("--") => i += 1,
            s => {
                dir = s.to_string();
                i += 1;
            }
        }
    }
    if mode == "new" {
        tasks_new(title);
        return;
    }
    let root = match fs::canonicalize(Path::new(&dir)) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    match mode {
        "check" => {
            let bad = task_check_problems(&root);
            if bad > 0 {
                eprintln!("-- {bad} task problem(s)");
                process::exit(1);
            }
            println!("tasks: clean");
        }
        "record" => tasks_record(&root, key, disposition, evidence, reason, record_digests),
        "rederive" => {
            if tasks_rederive(&root) > 0 {
                process::exit(1);
            }
        }
        _ => tasks_status(&root),
    }
}

/// `tasks --new "<title>"` emits the scaffolded task block: the heading with its slug anchor
/// at the end, and the empty field bullets to fill. The tool carries the anchor so the agent
/// authors only `depends`/`verify`/`inputs` (the fill-in-the-blank fold-back).
fn tasks_new(title: Option<String>) {
    let Some(title) = title else {
        eprintln!("usage: host-lifecycle tasks --new \"<title>\"");
        process::exit(2);
    };
    let slug = slugify(&title);
    if slug.is_empty() {
        eprintln!("host-lifecycle: `{title}` has no slug characters for an anchor");
        process::exit(2);
    }
    println!("### {title} {{#{slug}}}");
    println!();
    println!("- depends: ");
    println!("- verify: ");
    println!("- inputs: ");
}

fn tasks_status(root: &Path) {
    let docs = tracked_markdown(root).unwrap_or_default();
    let (tasks, problems) = parse_tasks(&docs);
    let receipts = read_task_receipts(root);
    let discharged: std::collections::HashSet<&str> = tasks
        .iter()
        .filter(|t| latest_task_receipt(&receipts, &t.key).is_some_and(|r| r.disposition == "done" || r.disposition == "skip"))
        .map(|t| t.key.as_str())
        .collect();
    println!("{} task(s) in the plan room:", tasks.len());
    for t in &tasks {
        let state = latest_task_receipt(&receipts, &t.key).map(|r| r.disposition.as_str()).unwrap_or("(no receipt)");
        let deps = if t.depends.is_empty() { "(root)".to_string() } else { t.depends.join(", ") };
        println!("  {} [{state}]  depends: {deps}", t.key);
    }
    let frontier: Vec<&str> = tasks
        .iter()
        .filter(|t| !discharged.contains(t.key.as_str()) && t.depends.iter().all(|d| discharged.contains(d.as_str())))
        .map(|t| t.key.as_str())
        .collect();
    if frontier.is_empty() {
        println!("ready frontier: (none — every task discharged, or blocked on an undischarged dep)");
    } else {
        println!("ready frontier (deps satisfied, may run in parallel): {}", frontier.join(", "));
    }
    for p in &problems {
        println!("note: {p}");
    }
}

fn tasks_record(root: &Path, key: Option<String>, disposition: Option<String>, evidence: Option<String>, reason: Option<String>, record_digests: bool) {
    let (Some(key), Some(disposition)) = (key, disposition) else {
        eprintln!("usage: host-lifecycle tasks --record <plan/NNNN#anchor> --disposition done|skip (--evidence <e> | --reason <call/NNNN>) [--record-digests] [<dir>]");
        process::exit(2);
    };
    let docs = match tracked_markdown(root) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("host-lifecycle: tasks: {e}");
            process::exit(2);
        }
    };
    let (tasks, _) = parse_tasks(&docs);
    let Some(task) = tasks.iter().find(|t| t.key == key) else {
        eprintln!("host-lifecycle: no task `{key}` (an anchored `### ` under `## Build sequence` in its plan README)");
        process::exit(2);
    };
    let (mut verify, mut inputs, mut digest) = (None, None, None);
    match disposition.as_str() {
        "done" => {
            if evidence.is_none() {
                eprintln!("host-lifecycle: a `done` task receipt needs `--evidence` (a re-derivable record)");
                process::exit(2);
            }
            verify = Some(match &task.verify {
                Some(TaskVerify::Mechanical(c)) => c.clone(),
                Some(TaskVerify::Attested(c)) => format!("attested {c}"),
                None => {
                    eprintln!("host-lifecycle: task `{key}` declares no `verify`; add one before recording done");
                    process::exit(2);
                }
            });
            if !task.inputs.is_empty() {
                inputs = Some(task.inputs.join(", "));
                if record_digests && matches!(task.verify, Some(TaskVerify::Mechanical(_))) {
                    match input_digest(&task.inputs, root) {
                        Ok(d) => digest = Some(d),
                        Err(e) => {
                            eprintln!("host-lifecycle: cannot fingerprint inputs: {e}");
                            process::exit(2);
                        }
                    }
                }
            }
        }
        "skip" => match &reason {
            Some(r) if valid_skip_citation(r) && cited_decision_exists(root, r) => {}
            _ => {
                eprintln!("host-lifecycle: a `skip` task receipt needs `--reason <call/NNNN>` resolving to a decision");
                process::exit(2);
            }
        },
        other => {
            eprintln!("host-lifecycle: unknown disposition `{other}` — use done or skip");
            process::exit(2);
        }
    }
    let r = TaskReceipt {
        key: key.clone(),
        disposition: disposition.clone(),
        verify,
        inputs,
        digest,
        evidence,
        reason,
        tool: Some(format!("host-lifecycle@{}", env!("CARGO_PKG_VERSION"))),
        recorded: Some(today()),
    };
    if let Err(e) = append_task_receipt(root, &r) {
        eprintln!("host-lifecycle: cannot write {TASK_RECEIPTS}: {e}");
        process::exit(2);
    }
    println!("recorded task receipt: {key} = {disposition}");
}

fn tasks_rederive(root: &Path) -> usize {
    let docs = tracked_markdown(root).unwrap_or_default();
    let (tasks, _) = parse_tasks(&docs);
    let receipts = read_task_receipts(root);
    let (mut refreshed, mut bad) = (0, 0);
    for t in &tasks {
        let Some(r) = latest_task_receipt(&receipts, &t.key) else { continue };
        if r.disposition != "done" {
            continue;
        }
        let Some(TaskVerify::Mechanical(cmd)) = &t.verify else { continue };
        if !verify_command_is_safe(cmd) {
            println!("HAZARD   {} — verify invokes the gate; refusing to run", t.key);
            bad += 1;
            continue;
        }
        if !run_recheck(root, cmd) {
            println!("FAILED   {} — `{cmd}` exited non-zero", t.key);
            bad += 1;
            continue;
        }
        let digest = if t.inputs.is_empty() {
            None
        } else {
            input_digest(&t.inputs, root).ok()
        };
        let fresh = TaskReceipt {
            key: t.key.clone(),
            disposition: "done".into(),
            verify: Some(cmd.clone()),
            inputs: (!t.inputs.is_empty()).then(|| t.inputs.join(", ")),
            digest,
            evidence: Some(format!("re-derived {}", today())),
            reason: None,
            tool: Some(format!("host-lifecycle@{}", env!("CARGO_PKG_VERSION"))),
            recorded: Some(today()),
        };
        match append_task_receipt(root, &fresh) {
            Ok(()) => {
                println!("ok       {} (re-derived, digest refreshed)", t.key);
                refreshed += 1;
            }
            // The fresh receipt is the persisted record of the re-derivation; a write that
            // fails leaves nothing persisted, so it is a failure, not a success (issue #8) —
            // the manual-record path treats the same error as fatal.
            Err(e) => {
                println!("FAILED   {} re-derived but cannot write receipt: {e}", t.key);
                bad += 1;
            }
        }
    }
    println!("-- {refreshed} re-derived, {bad} failed");
    bad
}

#[cfg(test)]
mod task_gate_tests {
    use super::*;

    fn doc(rel: &str, body: &str) -> (String, String) {
        (rel.to_string(), body.to_string())
    }

    fn tr(key: &str, disp: &str, verify: Option<&str>, inputs: Option<&str>, digest: Option<&str>, reason: Option<&str>) -> TaskReceipt {
        TaskReceipt {
            key: key.into(),
            disposition: disp.into(),
            verify: verify.map(Into::into),
            inputs: inputs.map(Into::into),
            digest: digest.map(Into::into),
            evidence: Some("ev".into()),
            reason: reason.map(Into::into),
            tool: None,
            recorded: None,
        }
    }

    #[test]
    fn task_receipt_ledger_roundtrips() {
        let r = tr("plan/0042#a", "done", Some("cargo test a"), Some("src/a.rs"), Some("deadbeef"), None);
        let parsed = parse_task_receipts(&task_receipt_stanza(&r));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].key, "plan/0042#a");
        assert_eq!(parsed[0].verify.as_deref(), Some("cargo test a"));
        assert_eq!(parsed[0].digest.as_deref(), Some("deadbeef"));
        // last-wins
        let two = format!("{}\n{}", task_receipt_stanza(&tr("plan/0042#a", "skip", None, None, None, Some("call/0024"))), task_receipt_stanza(&r));
        let parsed = parse_task_receipts(&two);
        assert_eq!(latest_task_receipt(&parsed, "plan/0042#a").unwrap().disposition, "done");
    }

    #[test]
    fn gate_checks_attested_mechanical_skip_missing_and_orphan() {
        let base = std::env::temp_dir().join(format!("hl-taskgate-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("call")).unwrap();
        process::Command::new("git").arg("-C").arg(&base).arg("init").arg("-q").output().unwrap();
        fs::write(base.join("call").join("0024-x.md"), "# x\n").unwrap();
        fs::write(base.join("engine.rs"), "fn main(){}\n").unwrap();

        let docs = vec![doc(
            "plan/0042-x/README.md",
            "## Build sequence\n\n### A {#a}\n- verify: attested call/0024\n\n### B {#b}\n- depends: #a\n- verify: cargo test b\n- inputs: engine.rs\n",
        )];
        let (tasks, problems) = parse_tasks(&docs);
        assert!(problems.is_empty(), "{problems:?}");
        let a = tasks.iter().find(|t| t.key.ends_with("#a")).unwrap();
        let b = tasks.iter().find(|t| t.key.ends_with("#b")).unwrap();

        // attested resolves → ok; an unresolved citation → hazard
        assert!(task_verdict(&base, a, &tr("plan/0042#a", "done", Some("attested call/0024"), None, None, None)).0);
        assert!(!task_verdict(&base, a, &tr("plan/0042#a", "done", Some("attested call/9999"), None, None, None)).0);

        // mechanical: fresh digest ok, drifted input stale, changed verify stale
        let dg = input_digest(&["engine.rs".to_string()], &base).unwrap();
        let rb = tr("plan/0042#b", "done", Some("cargo test b"), Some("engine.rs"), Some(&dg), None);
        assert!(task_verdict(&base, b, &rb).0, "fresh");
        fs::write(base.join("engine.rs"), "changed\n").unwrap();
        assert!(!task_verdict(&base, b, &rb).0, "stale input");
        assert!(!task_verdict(&base, b, &tr("plan/0042#b", "done", Some("cargo test OTHER"), Some("engine.rs"), Some(&dg), None)).0, "changed verify");

        // skip needs a resolvable reason
        assert!(task_verdict(&base, a, &tr("plan/0042#a", "skip", None, None, None, Some("call/0024"))).0);
        assert!(!task_verdict(&base, a, &tr("plan/0042#a", "skip", None, None, None, Some("call/9999"))).0);
        assert!(!task_verdict(&base, a, &tr("plan/0042#a", "skip", None, None, None, None)).0);
        // issue #7: a free-text reason is not a citation — the gate must match the recorder,
        // which requires `--reason <call/NNNN>`, so `wip` cannot slip a task through.
        assert!(!task_verdict(&base, a, &tr("plan/0042#a", "skip", None, None, None, Some("wip"))).0);

        // gate: only a is receipted. With plan/0042 OPEN, b is pending-ok; when complete it owes one.
        let receipts = vec![
            tr("plan/0042#a", "done", Some("attested call/0024"), None, None, None),
            tr("plan/0099#gone", "done", Some("attested operator"), None, None, None),
        ];
        let none: std::collections::HashSet<String> = std::collections::HashSet::new();
        let lines = task_gate(&base, &tasks, &receipts, &none);
        assert!(lines.iter().any(|l| l.label == "task plan/0042#b" && l.ok && l.note.contains("pending")), "b pending while open");
        let complete: std::collections::HashSet<String> = ["plan/0042".to_string()].into_iter().collect();
        let lines = task_gate(&base, &tasks, &receipts, &complete);
        assert!(lines.iter().any(|l| l.label == "task plan/0042#b" && !l.ok && l.note.contains("no receipt")), "b owes a receipt when complete");
        assert!(lines.iter().any(|l| l.label == "task plan/0042#a" && l.ok));
        // an orphan receipt is always a hazard, regardless of completion
        assert!(lines.iter().any(|l| l.label == "task plan/0099#gone" && !l.ok && l.note.contains("orphan")));

        let _ = fs::remove_dir_all(&base);
    }

    // issue #8: `tasks --rederive` must not report success when the verify passed but the
    // fresh receipt cannot be persisted — an unpersisted re-derivation is a failure.
    #[cfg(unix)]
    #[test]
    fn rederive_counts_an_unpersistable_receipt_as_a_failure() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("hl-rederive-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("plan/0001-x")).unwrap();
        let g = |args: &[&str]| {
            process::Command::new("git").arg("-C").arg(&base).args(args).output().unwrap();
        };
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        // a task with a trivial, safe mechanical verify, and a `done` receipt for it
        fs::write(base.join("plan/0001-x/README.md"), "## Build sequence\n\n### T {#t}\n- verify: true\n").unwrap();
        g(&["add", "-A"]);
        fs::write(base.join(TASK_RECEIPTS), task_receipt_stanza(&tr("plan/0001#t", "done", Some("true"), None, None, None))).unwrap();
        let canon = fs::canonicalize(&base).unwrap();
        // writable tree: the verify passes and the fresh receipt persists.
        assert_eq!(tasks_rederive(&canon), 0, "clean re-derive");
        // read-only root: reads still work, but the receipt write fails. A passing verify whose
        // receipt cannot be written must count as failed, not pass.
        let ro = fs::Permissions::from_mode(0o555);
        fs::set_permissions(&canon, ro).unwrap();
        let bad = tasks_rederive(&canon);
        fs::set_permissions(&canon, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(bad, 1, "an unpersistable receipt is a failure (issue #8)");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn milestone_complete_reads_the_status_section() {
        assert!(milestone_complete("## Status\n\ncomplete, landed 2026-06-24\n"));
        assert!(milestone_complete("## Status\n\n**DONE** (shipped)\n"));
        assert!(milestone_complete("# t\n## Status\nlanded\n"));
        assert!(!milestone_complete("## Status\n\nOpen, design phase.\n"));
        assert!(!milestone_complete("## Build sequence\n\ncomplete\n")); // not the Status section
        assert!(!milestone_complete("no status section here"));
        // issue #18: completion synonyms and a leading list marker are recognised.
        assert!(milestone_complete("## Status\n\n- shipped as v0.32.0\n"));
        assert!(milestone_complete("## Status\n\nReleased 2026-06-30\n"));
        assert!(milestone_complete("## Status\n\n+ complete\n")); // a `+` list marker is stripped
        // issue #11: a fenced `## Status` example does not flip the verdict. The real Status
        // says open, so a fenced "complete" example must not read as complete.
        assert!(!milestone_complete("## Status\n\nOpen.\n\n```\n## Status\ncomplete\n```\n"));
        // and a real Status after a fenced fake one is still read.
        assert!(milestone_complete("## Plan\n\n```\n## Status\nopen\n```\n\n## Status\n\ndone\n"));
    }
}

#[cfg(test)]
mod task_tests {
    use super::*;

    fn doc(rel: &str, body: &str) -> (String, String) {
        (rel.to_string(), body.to_string())
    }

    #[test]
    fn plan_id_of_reads_the_milestone_number() {
        assert_eq!(plan_id_of("plan/0042-receipted-task-graph/README.md").as_deref(), Some("plan/0042"));
        assert_eq!(plan_id_of("plan/0042-x/spec/foo.md"), None);
        assert_eq!(plan_id_of("STRUCTURE.md"), None);
        assert_eq!(plan_id_of("plan/PLAN.md"), None);
    }

    #[test]
    fn slugify_makes_a_clean_anchor() {
        assert_eq!(slugify("Run the migration"), "run-the-migration");
        assert_eq!(slugify("  Build & test (CI)!  "), "build-test-ci");
        assert_eq!(slugify("0042: ship it"), "0042-ship-it");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn heading_end_anchor_only_at_the_end_of_a_heading() {
        assert_eq!(heading_end_anchor("### Gather data {#gather-data}").as_deref(), Some("gather-data"));
        assert_eq!(heading_end_anchor("### {#gather} Gather data"), None); // start placement
        assert_eq!(heading_end_anchor("- not a heading {#x}").as_deref(), Some("x")); // anchor extraction is heading-agnostic; the parser gates on heading_level
        assert_eq!(heading_end_anchor("### plain"), None);
        assert_eq!(heading_end_anchor("### quoted `{#x}`"), None); // inside code span
    }

    #[test]
    fn parse_tasks_reads_a_cross_milestone_diamond_with_the_linear_default() {
        let docs = vec![
            doc(
                "plan/0050-engine/README.md",
                "# t\n\n## Build sequence\n\n### Build engine {#build-engine}\n\n- verify: cargo test engine\n- inputs: src/engine.rs\n",
            ),
            doc(
                "plan/0051-ship/README.md",
                "# t\n\n## Build sequence\n\n### Write tests {#write-cli-tests}\n\n- verify: cargo test cli\n\n### Ship the cli {#ship-cli}\n\n- depends: #write-cli-tests, plan/0050#build-engine\n- verify: attested call/0024\n",
            ),
        ];
        let (tasks, problems) = parse_tasks(&docs);
        assert!(problems.is_empty(), "structural problems: {problems:?}");
        assert_eq!(tasks.len(), 3);
        let ship = tasks.iter().find(|t| t.key.ends_with("#ship-cli")).unwrap();
        assert_eq!(ship.key, "plan/0051#ship-cli");
        assert_eq!(ship.depends, vec!["plan/0051#write-cli-tests", "plan/0050#build-engine"]);
        assert_eq!(ship.verify, Some(TaskVerify::Attested("call/0024".to_string())));
        let engine = tasks.iter().find(|t| t.key.ends_with("#build-engine")).unwrap();
        assert_eq!(engine.verify, Some(TaskVerify::Mechanical("cargo test engine".to_string())));
        assert_eq!(engine.inputs, vec!["src/engine.rs".to_string()]);
        assert!(engine.depends.is_empty()); // first task in its plan, a root
        // linear default: write-cli-tests is the first in plan/0051, so a root
        let wt = tasks.iter().find(|t| t.key.ends_with("#write-cli-tests")).unwrap();
        assert!(wt.depends.is_empty());
        // the graph is clean (every depends resolves, no cycle)
        assert!(task_graph_problems(&tasks).is_empty());
    }

    #[test]
    fn parse_tasks_treats_a_band_as_a_group_and_keeps_the_linear_default_across_it() {
        let docs = vec![doc(
            "plan/0090-x/README.md",
            "# t\n\n## Build sequence\n\n### Reader normalisers {#readers}\n\n- band\n\n### First task {#first}\n\n- verify: cargo test\n\n### Release {#release}\n\n- band\n\n### Second task {#second}\n\n- verify: cargo test\n",
        )];
        let (tasks, problems) = parse_tasks(&docs);
        assert!(problems.is_empty(), "a band-bearing plan is clean: {problems:?}");
        // the two bands push no task; only the two anchored tasks are nodes
        let keys: Vec<&str> = tasks.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, vec!["plan/0090#first", "plan/0090#second"], "bands are not tasks: {keys:?}");
        // the linear default chains #second onto #first across the Release band heading
        let second = tasks.iter().find(|t| t.key.ends_with("#second")).unwrap();
        assert_eq!(second.depends, vec!["plan/0090#first"], "chaining continues across a band: {:?}", second.depends);
        let first = tasks.iter().find(|t| t.key.ends_with("#first")).unwrap();
        assert!(first.depends.is_empty(), "the first task is a root even with a band before it: {:?}", first.depends);
        assert!(task_graph_problems(&tasks).is_empty());
    }

    #[test]
    fn linear_default_chains_consecutive_tasks() {
        let docs = vec![doc(
            "plan/0060-x/README.md",
            "## Build sequence\n\n### First {#first}\n- verify: a\n\n### Second {#second}\n- verify: b\n",
        )];
        let (tasks, _) = parse_tasks(&docs);
        let second = tasks.iter().find(|t| t.key.ends_with("#second")).unwrap();
        assert_eq!(second.depends, vec!["plan/0060#first"]);
    }

    #[test]
    fn disambiguation_flags_anchored_heading_outside_and_unanchored_inside() {
        let docs = vec![doc(
            "plan/0070-x/README.md",
            "## Design\n\n### A subsection {#a-sub}\n\n## Build sequence\n\n### Needs anchor\n\n- verify: a\n",
        )];
        let (tasks, problems) = parse_tasks(&docs);
        assert!(tasks.is_empty());
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("belongs under `## Build sequence`")));
        assert!(problems.iter().any(|p| p.contains("needs an anchor at its end")));
    }

    #[test]
    fn fenced_example_headings_are_not_tasks() {
        // a plan that shows the task syntax in a ``` example must not parse it as a task,
        // nor flag the example heading as misplaced.
        let docs = vec![doc(
            "plan/0071-x/README.md",
            "## Decision\n\nExample:\n\n```\n## Build sequence\n\n### Show me {#show-me}\n- verify: x\n```\n\n## Build sequence\n\n### Real task {#real-task}\n- verify: y\n",
        )];
        let (tasks, problems) = parse_tasks(&docs);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].key, "plan/0071#real-task");
    }

    #[test]
    fn graph_flags_dangling_and_cycle() {
        let dangling = vec![doc(
            "plan/0080-x/README.md",
            "## Build sequence\n\n### A {#a}\n- depends: #nope\n- verify: a\n",
        )];
        let (tasks, _) = parse_tasks(&dangling);
        let probs = task_graph_problems(&tasks);
        assert!(probs.iter().any(|p| p.contains("dangling dependency")), "{probs:?}");

        let cyclic = vec![doc(
            "plan/0081-x/README.md",
            "## Build sequence\n\n### A {#a}\n- depends: #b\n- verify: a\n\n### B {#b}\n- depends: #a\n- verify: b\n",
        )];
        let (tasks, _) = parse_tasks(&cyclic);
        let probs = task_graph_problems(&tasks);
        assert!(probs.iter().any(|p| p.contains("cycle")), "{probs:?}");
    }

    #[test]
    fn reference_integrity_flags_a_broken_prose_reference() {
        let docs = vec![doc(
            "plan/0090-x/README.md",
            "## Build sequence\n\n### A {#a}\n- verify: a\n\n## Notes\n\nThis follows plan/0090#ghost in spirit, and plan/0090#a is real.\n",
        )];
        let (tasks, _) = parse_tasks(&docs);
        let keys: std::collections::HashSet<String> = tasks.iter().map(|t| t.key.clone()).collect();
        let probs = task_reference_problems(&docs, &keys);
        assert_eq!(probs.len(), 1, "{probs:?}");
        assert!(probs[0].contains("plan/0090#ghost"));

        // a reference inside a fenced code block is illustrative, not flagged
        let fenced = vec![doc(
            "plan/0091-x/README.md",
            "## Build sequence\n\n### A {#a}\n- verify: a\n\n## Notes\n\n```\nexample: plan/0091#made-up\n```\n",
        )];
        let (tasks, _) = parse_tasks(&fenced);
        let keys: std::collections::HashSet<String> = tasks.iter().map(|t| t.key.clone()).collect();
        assert!(task_reference_problems(&fenced, &keys).is_empty(), "fenced example must not flag");
    }
}

fn software(args: &[String]) {
    let mut mode: Option<&str> = None;
    let mut pos: Vec<&String> = Vec::new();
    // `--item <name>[@<branch>]` narrows the operation to one component (plan/0029);
    // a flag, not a positional, so it never collides with the `<dir>` positional.
    let mut item: Option<&str> = None;
    // `--lock <name>` graduates one onboarding component: writes its `deps-bundle.lock`
    // from the recorded pin and drives its release (plan/0057). The name is the flag's
    // value, not a positional, so it never collides with the `<dir>` positional.
    let mut lock_name: Option<&str> = None;
    // `--teardown` removes a component's materialized worktrees + bare store;
    // `--force` overrides the unsaved-work guard (plan/0029).
    let mut force = false;
    // `--partial` clones the bare store with `--filter=blob:none` (host-lifecycle#14):
    // a partial clone still lands every commit and tree, so the pin reproduces, but
    // blobs are fetched on demand at worktree checkout instead of the whole history up
    // front. Opt-in, so an adopter on a remote without partial-clone support is unaffected.
    let mut partial = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--materialize" => mode = Some("materialize"),
            "--check" => mode = Some("check"),
            "--verify-build" => mode = Some("verify-build"),
            "--install-hooks" => mode = Some("install-hooks"),
            "--verify-setup" => mode = Some("verify-setup"),
            "--teardown" => mode = Some("teardown"),
            "--partial" => partial = true,
            "--lock" => {
                mode = Some("lock");
                let Some(v) = args.get(i + 1) else {
                    eprintln!("host-lifecycle: --lock needs <name>");
                    process::exit(2);
                };
                lock_name = Some(v.as_str());
                i += 1;
            }
            "--force" => force = true,
            "--item" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("host-lifecycle: --item needs <name>[@<branch>]");
                    process::exit(2);
                };
                item = Some(v.as_str());
                i += 1;
            }
            _ => pos.push(&args[i]),
        }
        i += 1;
    }
    let Some(dir) = pos.first() else {
        eprintln!("host-lifecycle software <--materialize|--check|--verify-build|--verify-setup|--install-hooks|--teardown|--lock <name>> [--item <name>[@<branch>]] [--force] [--partial] <dir>");
        process::exit(2);
    };
    let Some(mode) = mode else {
        eprintln!("host-lifecycle software needs --materialize, --check, --verify-build, --verify-setup, --install-hooks, --teardown, or --lock <name>");
        process::exit(2);
    };
    let root = match fs::canonicalize(Path::new(dir.as_str())) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    let mut recipe = load_software(&root);
    if recipe.is_empty() {
        eprintln!("host-lifecycle: no [software \"<name>\"] stanzas in {SOFTWARE}");
        process::exit(2);
    }
    if let Some(spec) = item {
        recipe = filter_item(recipe, spec);
    }
    match mode {
        "materialize" => software_materialize(&root, &recipe, partial),
        "check" => {
            let mut owed: Vec<String> = Vec::new();
            let bad = software_check_owed(&root, &recipe, &mut owed);
            if bad > 0 {
                eprintln!("-- {bad} item(s) need attention");
                process::exit(1);
            }
            // Owed graduations are advisory, not a fault (turning onboarding red is the
            // retro-red trap plan/0051 rejected, and it would block a release gate that
            // re-runs `--check`). But the owed work must not hide behind a bare green: a
            // counted, enumerated summary that names the next action re-lists it every
            // run, so a cold read surfaces it (plan/0057, Bly's over-report direction).
            if !owed.is_empty() {
                println!(
                    "-- {} deps-bundle graduation(s) owed: {}; run: host-lifecycle software --lock <name> {dir}",
                    owed.len(),
                    owed.join(", ")
                );
            }
            println!("-- all components at their pinned SHA; no worktree-symlink hazards");
        }
        "verify-build" => software_verify_build(&root, &recipe),
        "install-hooks" => software_install_hooks(&root, &recipe),
        "verify-setup" => process::exit(setup::verify_setup(&root, &recipe)),
        "teardown" => software_teardown(&root, &recipe, force),
        "lock" => software_lock(&root, &recipe, lock_name.expect("--lock sets lock_name")),
        _ => unreachable!(),
    }
}

/// `software --lock <name>` graduates one onboarding component to attested (plan/0057):
/// it writes the producer's `deps-bundle.lock` from the recorded pin (never hand-typed),
/// stages it, and drives a fix-only release. The authority is release-grade: the committed lock
/// advances the producer HEAD, so `.host-software` keeps pinning a released, tagged commit
/// (dual-release-authority unchanged). Fails loud on the wrong state; a no-op on an
/// already-locked component, so the verb is idempotent.
fn software_lock(root: &Path, recipe: &[Software], name: &str) {
    let Some(s) = recipe.iter().find(|s| s.name == name) else {
        eprintln!("host-lifecycle: no component `{name}` in {SOFTWARE}");
        process::exit(2);
    };
    let Some((url, sha)) = &s.deps_bundle else {
        eprintln!("host-lifecycle: {name} declares no deps-bundle — nothing to lock");
        process::exit(2);
    };
    let work = worktree_dir(root, &s.name, &s.branch);
    if !work.is_dir() {
        eprintln!(
            "host-lifecycle: {name} is not materialized at {} — run `software --materialize` first",
            work.display()
        );
        process::exit(2);
    }
    // Fail loud on the wrong state; the onboarding (no-lock) case is the one that proceeds.
    let lock = work.join("deps-bundle.lock");
    if let Ok(text) = fs::read_to_string(&lock) {
        let f: Vec<&str> = text.split_whitespace().collect();
        if f.len() >= 2 && f[0] == url && f[1] == sha {
            println!("ok       {name} deps-bundle.lock already matches the pin — already locked, nothing to do");
            return;
        }
        eprintln!("host-lifecycle: {name} deps-bundle.lock differs from the recorded pin (producer drift) — resolve the drift; --lock will not overwrite a disagreeing lock");
        process::exit(1);
    }
    // Write the lock from the recorded pin (so the content is never hand-typed) and stage it.
    let content = format!("{url} {sha}\n");
    if let Err(e) = fs::write(&lock, &content) {
        eprintln!("host-lifecycle: cannot write {}: {e}", lock.display());
        process::exit(2);
    }
    println!("  wrote {name}/deps-bundle.lock ({url} {})", short(sha));
    if !git_ok(&work, &["add", "deps-bundle.lock"]) {
        eprintln!("host-lifecycle: could not stage deps-bundle.lock in {}", work.display());
        process::exit(1);
    }
    println!("  staged deps-bundle.lock");
    println!("\n{name}: locking is release-grade; the committed lock advances the producer, so it ships as a fix-only release. The release commit below includes the staged lock.");
    // Drive the fix-only release: the verify gate, the version bump, the container rebuild
    // (the artifact re-derives byte-identically — the lock is never compiled in), and the
    // operator-run outward steps (commit the lock + bump, tag, push, re-pin, receipt).
    run_release(root, name, Some("neither"), false);
}

/// `--install-hooks`: for each component with a `hooks` script, copy it into
/// `.git/hooks` as `pre-commit` and `commit-msg`, alongside the verified deploy
/// artifact (the binary the dispatch script invokes). Closes the fresh-clone gap
/// where the worktree and skill symlink were materialized but the commit hooks
/// were not. Exits non-zero if any component with `hooks` cannot be installed.
fn software_install_hooks(root: &Path, recipe: &[Software]) {
    let (installed, failed) = install_hooks(root, recipe);
    if installed == 0 && failed == 0 {
        println!("no components declare a `hooks` script; nothing to install");
    }
    // A state change, not a lifecycle event: the installed binary's hash moved, so the
    // fingerprint is refreshed and no receipt is appended (plan/0074).
    envhash::write_envhash(root, recipe);
    if failed > 0 {
        process::exit(1);
    }
}

/// Every hooks directory this install must cover: the host repository's, plus one
/// per materialized worktree. A worktree is a real commit surface — commits land in
/// it directly — so leaving it ungated let a tell through the one door the host
/// repo's hook never watches (plan/0074, Bug A). Deduplicated by resolved path,
/// because git shares one hooks directory across a store's worktrees, and skipping
/// worktrees that are absent, so a host that materialized less is not a failure.
fn hook_targets(root: &Path, recipe: &[Software]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let add = |out: &mut Vec<PathBuf>, d: Option<PathBuf>| {
        if let Some(d) = d {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    };
    add(&mut out, git_hooks_dir(root));
    for s in recipe {
        let mut branches: Vec<String> = vec![s.branch.clone()];
        branches.extend(s.worktrees.iter().cloned());
        branches.extend(s.lines.iter().map(|w| w.branch.clone()));
        for b in branches {
            let wt = worktree_dir(root, &s.name, &b);
            if wt.is_dir() {
                add(&mut out, git_hooks_dir(&wt));
            }
        }
    }
    out
}

/// The install loop, factored out to return `(installed, failed)` counts instead
/// of exiting — keeps it testable. `installed` counts gated targets, so one
/// component installing into the host repo and two worktrees counts three.
fn install_hooks(root: &Path, recipe: &[Software]) -> (usize, usize) {
    let targets = hook_targets(root, recipe);
    if targets.is_empty() {
        eprintln!("host-lifecycle: not a git repository: {}", root.display());
        return (0, 1);
    }
    for t in &targets {
        if let Err(e) = fs::create_dir_all(t) {
            eprintln!("host-lifecycle: cannot create {}: {e}", t.display());
            return (0, 1);
        }
    }
    let mut installed = 0;
    let mut failed = 0;
    for s in recipe {
        let Some(hooks_rel) = &s.hooks else { continue };
        let worktree = worktree_dir(root, &s.name, &s.branch);
        let script = worktree.join(hooks_rel);
        if !script.is_file() {
            println!("MISSING  {}/{hooks_rel} (run --materialize)", s.name);
            failed += 1;
            continue;
        }
        // The worktree must be at its recorded pin so the installed binary is
        // built from the audited source. The binary's exact bytes need not match
        // the recorded canonical hash — that hash is the pinned container's
        // output; a local toolchain legitimately differs. The hash match is
        // reported as an informational note, not a gate.
        let head = git_out(&worktree, &["rev-parse", "HEAD"]);
        if head.is_none() || head != git_out(&worktree, &["rev-parse", &s.pin]) {
            println!("WORKTREE {} not at its pin (run --materialize)", s.name);
            failed += 1;
            continue;
        }
        let Some((art_path, want)) = &s.artifact else {
            println!("SKIP     {} (hooks set but no artifact to install)", s.name);
            failed += 1;
            continue;
        };
        let bin = worktree.join(art_path);
        if !bin.is_file() {
            println!("MISSING  {}/{art_path} — build the component first ({})", s.name,
                s.build.as_deref().unwrap_or("cargo build --release --locked"));
            failed += 1;
            continue;
        }
        let provenance = if sha256_file(&bin).as_deref() == Some(want.as_str()) {
            "verified against the canonical hash"
        } else {
            "local build (differs from the canonical hash)"
        };
        let bin_name = Path::new(art_path).file_name().unwrap_or_default();
        for hooks_dir in &targets {
            let installs = [
                (script.as_path(), hooks_dir.join("pre-commit")),
                (script.as_path(), hooks_dir.join("commit-msg")),
                (bin.as_path(), hooks_dir.join(bin_name)),
            ];
            let mut ok = true;
            for (src, dst) in installs {
                if let Err(e) = copy_executable(src, &dst) {
                    println!("FAIL     {} -> {}: {e}", src.display(), dst.display());
                    ok = false;
                }
            }
            if ok {
                println!("OK       {} hooks installed in {} (pre-commit, commit-msg, {}) — {provenance}",
                    s.name, hooks_dir.display(), bin_name.to_string_lossy());
                installed += 1;
            } else {
                failed += 1;
            }
        }
    }
    (installed, failed)
}

/// The repo's actual hooks directory, via `git rev-parse --git-path hooks` (so a
/// worktree or a custom `core.hooksPath` resolves correctly). Relative paths are
/// resolved against `root`.
fn git_hooks_dir(root: &Path) -> Option<PathBuf> {
    let o = process::Command::new("git")
        .arg("-C").arg(root)
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if p.is_empty() {
        return None;
    }
    let path = Path::new(&p);
    Some(if path.is_absolute() { path.to_path_buf() } else { root.join(path) })
}

/// Copy a file and mark it executable (0o755 on Unix).
fn copy_executable(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::copy(src, dst)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dst, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// The first available OCI runtime (`docker`, then `podman`), or `None`. Probed by
/// `<rt> version`, which contacts the daemon — so a present client with a stopped
/// daemon counts as unavailable, and `--verify-build` then skips rather than failing.
fn container_runtime() -> Option<&'static str> {
    ["docker", "podman"].into_iter().find(|rt| {
        process::Command::new(rt)
            .arg("version")
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Merge a `cargo vendor` `[source.*]` config snippet into an existing
/// `.cargo/config.toml` body, preserving the existing content (e.g. the reproducible
/// build-id rustflags). Pure so it is unit-testable; the staging step writes the result.
fn merge_vendor_config(existing: &str, snippet: &str) -> String {
    let mut out = existing.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(snippet.trim());
    out.push('\n');
    out
}

/// Stage a pinned dependency bundle into `work` for an offline build (plan/0032): the
/// one controlled, pinned network fetch. Download the tarball, verify its sha256 against
/// `want_sha` (the provenance half of the hermeticity gate — refuse a byte until it
/// matches), extract it (it ships `vendor/` plus a `vendor-config.toml` source snippet),
/// and merge the snippet into `work/.cargo/config.toml` preserving any existing rustflags.
/// `--network none` at build time is the egress half. Returns Err with a message on any
/// failure, so the caller can DRIFT/block rather than fall back to a networked build.
fn stage_deps_bundle(work: &Path, url: &str, want_sha: &str) -> Result<(), String> {
    let tarball = work.join(".host-deps-bundle.tar.gz");
    let ok = process::Command::new("curl")
        .args(["-fsSL", "--retry", "3", "--retry-delay", "2", "-o"])
        .arg(&tarball)
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err(format!("could not download deps-bundle from {url}"));
    }
    match sha256_file(&tarball) {
        Some(h) if h == want_sha => {}
        Some(h) => return Err(format!("deps-bundle sha mismatch: built {h}, recorded {want_sha}")),
        None => return Err("deps-bundle sha could not be computed".to_string()),
    }
    let ok = process::Command::new("tar")
        .arg("xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(work)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = fs::remove_file(&tarball);
    if !ok {
        return Err("could not extract deps-bundle".to_string());
    }
    let snippet_path = work.join("vendor-config.toml");
    let snippet = fs::read_to_string(&snippet_path)
        .map_err(|e| format!("deps-bundle has no vendor-config.toml: {e}"))?;
    let cfg_dir = work.join(".cargo");
    let cfg = cfg_dir.join("config.toml");
    let existing = fs::read_to_string(&cfg).unwrap_or_default();
    fs::create_dir_all(&cfg_dir).map_err(|e| format!("cannot create {}: {e}", cfg_dir.display()))?;
    fs::write(&cfg, merge_vendor_config(&existing, &snippet))
        .map_err(|e| format!("cannot write {}: {e}", cfg.display()))?;
    let _ = fs::remove_file(&snippet_path);
    Ok(())
}

/// Reverts a deps-bundle staged into the *live* canonical worktree (issue #22): it removes the
/// extracted `vendor/` dir and restores the pre-staging `.cargo/config.toml` (the original text,
/// or removes the file if there was none). `release` calls `restore` explicitly after the build,
/// but the `Drop` is the backstop — an abnormal exit (a panic) between staging and that explicit
/// restore still leaves the worktree carrying only the version bump, never the source-replacement
/// snippet and vendored tree. `restore` disarms, so it never repeats on the subsequent drop.
struct StagedDepsGuard {
    work: PathBuf,
    cfg_backup: Option<String>,
    armed: bool,
}

impl StagedDepsGuard {
    fn restore(&mut self) {
        if !self.armed {
            return;
        }
        self.armed = false;
        let _ = fs::remove_dir_all(self.work.join("vendor"));
        let cfg = self.work.join(".cargo/config.toml");
        match &self.cfg_backup {
            Some(text) => {
                let _ = fs::write(&cfg, text);
            }
            None => {
                let _ = fs::remove_file(&cfg);
            }
        }
    }
}

impl Drop for StagedDepsGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Run a recorded `build` recipe inside the recorded `toolchain` container against
/// `src` (mounted at `/src`), returning whether it succeeded. Shared by
/// `--verify-build` and `release` so the docker incantation lives in one place. The
/// container is root and writes root-owned output into the mount, so the recipe chowns
/// `/src` back to the mount owner before exiting, keeping the worktree removable. When
/// `offline` (a component pins a `deps-bundle`), the container runs `--network none` so
/// the build is provably network-free; otherwise an optional `HOST_LIFECYCLE_DOCKER_NETWORK`
/// (e.g. `host`) covers environments whose default docker bridge lacks DNS.
fn run_build_in_container(runtime: &str, image: &str, build: &str, src: &Path, offline: bool) -> bool {
    let src_abs = fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
    let wrapped =
        format!("{build}; rc=$?; chown -R \"$(stat -c '%u:%g' /src)\" /src 2>/dev/null || true; exit $rc");
    let mut cmd = process::Command::new(runtime);
    cmd.arg("run").arg("--rm");
    if offline {
        cmd.arg("--network").arg("none");
    } else if let Ok(net) = env::var("HOST_LIFECYCLE_DOCKER_NETWORK") {
        if !net.is_empty() {
            cmd.arg("--network").arg(net);
        }
    }
    cmd.arg("-v")
        .arg(format!("{}:/src", src_abs.to_string_lossy()))
        .arg("-w")
        .arg("/src")
        .arg(image)
        .arg("sh")
        .arg("-c")
        .arg(&wrapped);
    cmd.status().map(|st| st.success()).unwrap_or(false)
}

/// `--verify-build`: prove reproducibility (the heavy lane). For each component with a
/// `build` recipe, materialize a clean throwaway worktree at the pin, run the recorded
/// build **inside the recorded `toolchain` container** (host#14 — never the ambient
/// rust), hash the `artifact`, and compare to the recorded sha. A `repro-exempt`
/// component citing a real decision is reported (warn) and its rebuild skipped — the
/// escape clause for not-yet-reproducible migrated software (issue #10).
fn software_verify_build(root: &Path, recipe: &[Software]) {
    let mut bad = 0usize;
    // plan/0052 (no-hollow-green): three states the summary and the exit code must agree
    // on — VERIFIED (rebuilt and matched), DEFERRED/EXEMPT (legitimately not checked here),
    // and UNVERIFIABLE (in scope here but the lane could not run). The clean attestation is
    // printed only when a build was actually verified and none was UNVERIFIABLE; an
    // all-skipped or could-not-check run never reports clean.
    let mut verified = 0usize;
    let mut deferred = 0usize;
    let mut exempt = 0usize;
    let mut unverifiable = 0usize;
    let host = std::env::consts::OS;
    for s in recipe {
        let bare = store_dir(root, &s.name);
        for b in s.builds_view() {
            let tag = match b.platform {
                Some(p) => format!("{} [{}]", s.name, p),
                None => s.name.clone(),
            };
            let Some((path, sha)) = b.artifact else {
                println!("skip     {tag} (no artifact recorded)");
                continue;
            };
            // A build reproduces only on its `attest-host` (it cannot run a foreign
            // toolchain here) — skipped like an exemption, not failed.
            if let Some(ah) = b.attest_host {
                if ah != host {
                    println!("DEFERRED {tag} reproduces on {ah} (host is {host}) — verified there, not here");
                    deferred += 1;
                    continue;
                }
            }
            if let Some(cite) = b.repro_exempt {
                if cited_decision_exists(root, cite) {
                    println!("EXEMPT   {tag} repro-exempt ({cite}) — rebuild comparison skipped");
                    exempt += 1;
                } else {
                    println!("DRIFT    {tag} repro-exempt cites missing decision {cite}");
                    bad += 1;
                }
                continue;
            }
            let Some(build) = b.build else {
                println!("DRIFT    {tag} has an artifact but no `build` recipe to reproduce it");
                bad += 1;
                continue;
            };
            // host#14: a reproducible build is only meaningful in the *recorded*
            // toolchain. Build inside the digest-pinned `toolchain` container, never
            // the ambient rust — which legitimately differs and yields a false DRIFT.
            // Honor each component's own recorded image verbatim (no version is
            // imposed). With no pin or no runtime, skip clearly — never ambient-build.
            let Some(image) = b.toolchain else {
                println!("UNVERIFIABLE {tag} — no `toolchain` pin; cannot verify in a pinned environment. Record a toolchain, or run on the attest-host (software --check also flags this)");
                unverifiable += 1;
                continue;
            };
            let Some(runtime) = container_runtime() else {
                println!("UNVERIFIABLE {tag} — no container runtime (docker/podman); cannot rebuild in the recorded toolchain {image}. Install docker or podman, or run on the attest-host");
                unverifiable += 1;
                continue;
            };
            if !bare.is_dir() {
                println!("MISSING  software/{}/.bare (run --materialize)", s.name);
                bad += 1;
                continue;
            }
            // A per-platform verify worktree under the component dir, so concurrent
            // platform builds of the same source pin do not collide on one tree.
            let suffix = b.platform.map(|p| format!("-{p}")).unwrap_or_default();
            let work = component_dir(root, &s.name).join(format!(".host-verify{suffix}"));
            let _ = fs::remove_dir_all(&work);
            let work_s = work.to_string_lossy().to_string();
            if !git_ok(&bare, &["worktree", "add", "--detach", &work_s, &s.pin]) {
                println!("ERROR    {tag} — cannot create a verify worktree at {}", short(&s.pin));
                bad += 1;
                continue;
            }
            // plan/0032: a pinned dependency bundle makes the build hermetic. Stage it
            // into the throwaway worktree (download, verify the recorded sha, extract,
            // merge the source config) and build under `--network none`. The sha check is
            // the provenance half of the gate; the removed worktree needs no cleanup.
            let offline = s.deps_bundle.is_some();
            if let Some((url, want)) = &s.deps_bundle {
                if let Err(e) = stage_deps_bundle(&work, url, want) {
                    println!("DRIFT    {tag} deps-bundle: {e}");
                    bad += 1;
                    let _ = git_ok(&bare, &["worktree", "remove", "--force", &work_s]);
                    let _ = fs::remove_dir_all(&work);
                    continue;
                }
            }
            // Run the recorded recipe inside the recorded image, never the ambient rust.
            let built = run_build_in_container(runtime, image, build, &work, offline);
            if !built {
                println!("ERROR    {tag} — build failed in {runtime} {image}: `{build}`");
                bad += 1;
            } else {
                let rebuilt = sha256_file(&work.join(path));
                match artifact_reproduces(rebuilt.as_deref(), sha) {
                    Ok(()) => {
                        println!("ok       {tag} rebuild reproduces {path} @ {} (in {image})", short(sha));
                        verified += 1;
                    }
                    Err(reason) => {
                        println!("DRIFT    {tag} {reason}");
                        bad += 1;
                    }
                }
            }
            let _ = git_ok(&bare, &["worktree", "remove", "--force", &work_s]);
            let _ = fs::remove_dir_all(&work);
        }
    }
    if bad > 0 {
        eprintln!("-- {bad} build(s) failed reproducibility verification");
        process::exit(1);
    }
    // No-hollow-green: never attest what was not checked. An in-scope build the lane
    // could not run (UNVERIFIABLE) makes the run incomplete; the clean line is printed
    // only when a build was actually verified here and none was UNVERIFIABLE.
    if unverifiable > 0 {
        eprintln!("-- verify-build INCOMPLETE: {verified} verified, {deferred} deferred, {exempt} exempt, {unverifiable} UNVERIFIABLE — reproducibility NOT attested");
        process::exit(1);
    }
    // The rebuild ran in the recorded toolchain, which pulls the image if it was
    // absent: the image digest dimension is the one this op moves, so it refreshes
    // the fingerprint and, like every state change, appends no receipt (plan/0074).
    envhash::write_envhash(root, recipe);
    if verified > 0 {
        println!("-- {verified} build(s) reproduced their recorded artifact ({deferred} deferred, {exempt} exempt)");
    } else {
        println!("-- 0 builds verified here ({deferred} deferred to other hosts, {exempt} exempt); nothing to attest");
    }
}

/// The reproducibility verdict for a rebuilt artifact: `Ok` when the rebuilt hash equals
/// the recorded one, `Err(reason)` (a DRIFT) otherwise, and `Err` for a missing artifact.
/// Pure, so the unreproduced-artifact detection is unit-testable without a container
/// rebuild (plan/0051 finding 4 — the rule was dispositioned to a test that asserted it
/// did NOT bite, so a regression accepting a non-matching rebuild would have stayed green).
fn artifact_reproduces(rebuilt: Option<&str>, recorded: &str) -> Result<(), String> {
    match rebuilt {
        Some(h) if h == recorded => Ok(()),
        Some(h) => Err(format!("rebuild is {} but recorded {} — NOT reproducible", short(h), short(recorded))),
        None => Err("built artifact not found at the recorded path".to_string()),
    }
}

/// Read `.host-software` from the repo root, exiting if it is absent.
fn load_software(root: &Path) -> Vec<Software> {
    match fs::read_to_string(root.join(SOFTWARE)) {
        Ok(t) => parse_software(&t),
        Err(e) => {
            eprintln!("host-lifecycle: cannot read {SOFTWARE}: {e}");
            process::exit(2);
        }
    }
}

/// Parse the git-config-style recipe: a `[software "<name>"]` header opens a
/// stanza; `url`, `pin`, and a space-separated `worktrees` list follow. Unknown
/// keys are ignored; a stanza missing `url` or `pin` is fatal (not materialisable).
fn parse_software(text: &str) -> Vec<Software> {
    let mut out: Vec<Software> = Vec::new();
    // While inside a `[build "s" "p"]` subsection, key=val lines configure the
    // current platform build rather than the software stanza.
    let mut in_build = false;
    // Normalize a value token: strip a `"..."` wrapper, exit loudly on a stray quote (issue #6).
    // Applied per token so multi-token fields (`artifact = <path> <sha>`) unquote each side.
    let unq = |tok: &str, i: usize| -> String {
        unquote_recipe_token(tok).unwrap_or_else(|| {
            eprintln!("host-lifecycle: {SOFTWARE}:{}: value `{tok}` has an unbalanced or stray quote (recipe values are bare; only the [software \"…\"] name is quoted)", i + 1);
            process::exit(2);
        })
    };
    // The free-form `build` command is passed verbatim to a shell, where interior quotes are
    // meaningful (`CFLAGS="-O2" make`); strip only a clean surrounding wrapper and pass anything
    // else through, never failing on an interior quote (issue #6 review). Contrast `unq`, which
    // fails closed because a ref/path/hash/URL is bare by convention.
    let unq_cmd = |tok: &str| unquote_recipe_token(tok).unwrap_or_else(|| tok.to_string());
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(name) = t.strip_prefix("[software \"").and_then(|r| r.strip_suffix("\"]")) {
            out.push(Software {
                name: name.to_string(),
                url: String::new(),
                pin: String::new(),
                branch: "main".to_string(),
                worktrees: Vec::new(),
                lines: Vec::new(),
                build: None,
                toolchain: None,
                deploy: None,
                artifact: None,
                repro_exempt: None,
                hooks: None,
                deps_bundle: None,
                builds: Vec::new(),
            });
            in_build = false;
            continue;
        }
        // `[build "<software>" "<platform>"]` — a per-platform build nested under
        // the matching software stanza (issue #1).
        if let Some(inner) = t.strip_prefix("[build \"").and_then(|r| r.strip_suffix("\"]")) {
            let Some((soft, plat)) = inner.split_once("\" \"") else {
                eprintln!("host-lifecycle: {SOFTWARE}:{}: `[build]` needs `\"<software>\" \"<platform>\"`", i + 1);
                process::exit(2);
            };
            let Some(cur) = out.last_mut() else {
                eprintln!("host-lifecycle: {SOFTWARE}:{}: [build] before any [software \"...\"] stanza", i + 1);
                process::exit(2);
            };
            if cur.name != soft {
                eprintln!("host-lifecycle: {SOFTWARE}:{}: [build \"{soft}\" …] must follow [software \"{soft}\"]", i + 1);
                process::exit(2);
            }
            cur.builds.push(PlatformBuild {
                platform: plat.to_string(),
                build: None,
                toolchain: None,
                deploy: None,
                artifact: None,
                repro_exempt: None,
                attest_host: None,
            });
            in_build = true;
            continue;
        }
        let Some((key, val)) = t.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());
        let Some(cur) = out.last_mut() else {
            eprintln!("host-lifecycle: {SOFTWARE}:{}: `{key}` before any [software \"...\"] stanza", i + 1);
            process::exit(2);
        };
        if in_build {
            let b = cur.builds.last_mut().expect("in_build implies a pushed build");
            match key {
                "build" => b.build = Some(unq_cmd(val)),
                "toolchain" => b.toolchain = Some(unq(val, i)),
                "deploy" => b.deploy = Some(unq(val, i)),
                "artifact" => {
                    let f: Vec<&str> = val.split_whitespace().collect();
                    let [path, sha] = f[..] else {
                        eprintln!("host-lifecycle: {SOFTWARE}:{}: `artifact` needs `<path> <sha256>`", i + 1);
                        process::exit(2);
                    };
                    b.artifact = Some((unq(path, i), unq(sha, i)));
                }
                "repro-exempt" => b.repro_exempt = Some(unq(val, i)),
                "attest-host" => b.attest_host = Some(unq(val, i)),
                _ => {}
            }
            continue;
        }
        match key {
            "url" => cur.url = unq(val, i),
            "pin" => cur.pin = unq(val, i),
            "branch" => cur.branch = unq(val, i),
            "worktrees" => cur.worktrees = val.split_whitespace().map(|t| unq(t, i)).collect(),
            "worktree" => {
                // `worktree = <branch> <pin> [store=<path>] [host=<os>]` — a parallel
                // line, fully pinned; the path is `software/<name>/<branch>/`; optional
                // external store + OS gate (plan/0029 retired the leading <dir> token).
                let f: Vec<&str> = val.split_whitespace().collect();
                if f.len() < 2 {
                    eprintln!(
                        "host-lifecycle: {SOFTWARE}:{}: `worktree` needs `<branch> <pin> [store=<path>] [host=<os>]`",
                        i + 1
                    );
                    process::exit(2);
                }
                let (branch, pin) = (unq(f[0], i), unq(f[1], i));
                let mut store = None;
                let mut host = None;
                for tok in &f[2..] {
                    if let Some(v) = tok.strip_prefix("store=") {
                        store = Some(unq(v, i));
                    } else if let Some(v) = tok.strip_prefix("host=") {
                        host = Some(unq(v, i));
                    } else {
                        eprintln!("host-lifecycle: {SOFTWARE}:{}: unknown `worktree` token `{tok}` (expected store=/host=)", i + 1);
                        process::exit(2);
                    }
                }
                cur.lines.push(Worktree {
                    branch,
                    pin,
                    store,
                    host,
                });
            }
            "build" => cur.build = Some(unq_cmd(val)),
            "toolchain" => cur.toolchain = Some(unq(val, i)),
            "deploy" => cur.deploy = Some(unq(val, i)),
            "artifact" => {
                // `artifact = <path> <sha256>` — the deployed artifact's expected hash.
                let f: Vec<&str> = val.split_whitespace().collect();
                let [path, sha] = f[..] else {
                    eprintln!("host-lifecycle: {SOFTWARE}:{}: `artifact` needs `<path> <sha256>`", i + 1);
                    process::exit(2);
                };
                cur.artifact = Some((unq(path, i), unq(sha, i)));
            }
            "repro-exempt" => cur.repro_exempt = Some(unq(val, i)),
            "hooks" => cur.hooks = Some(unq(val, i)),
            "deps-bundle" => {
                // `deps-bundle = <url> <sha256>` — a pinned vendored-dependency bundle.
                let f: Vec<&str> = val.split_whitespace().collect();
                let [url, sha] = f[..] else {
                    eprintln!("host-lifecycle: {SOFTWARE}:{}: `deps-bundle` needs `<url> <sha256>`", i + 1);
                    process::exit(2);
                };
                cur.deps_bundle = Some((unq(url, i), unq(sha, i)));
            }
            _ => {}
        }
    }
    for s in &out {
        if s.url.is_empty() || s.pin.is_empty() {
            eprintln!("host-lifecycle: {SOFTWARE}: [software \"{}\"] needs both url and pin", s.name);
            process::exit(2);
        }
    }
    let problems = recipe_problems(&out);
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("host-lifecycle: {SOFTWARE}: {p}");
        }
        process::exit(2);
    }
    out
}

/// True if `dir` (a recorded worktree path) would escape the host root: an
/// absolute path, or one whose `..` components climb above the root. Purely
/// lexical (no filesystem access), so it catches the wrong-tree footgun before
/// materialize. The sanctioned way to back a worktree with an off-tree store is a
/// `store=` line, whose `dir` is still an in-tree handle (issue #2).
fn escapes_root(dir: &str) -> bool {
    let p = Path::new(dir);
    let mut depth: i32 = 0;
    for c in p.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            // RootDir / Prefix → absolute, always escapes.
            _ => return true,
        }
    }
    false
}

/// Recipe-level defects detectable without the filesystem: a duplicate `[software "<name>"]`
/// stanza (issue #16 — materialize and release act on the first, so the second's url is
/// silently ignored), and an `artifact` or `hooks` path that escapes its worktree (issue #21
/// — an absolute artifact path replaces the join base, so `--verify-build` would hash a file
/// outside the throwaway worktree). Pure, so it is unit-tested; `parse_software` exits on any.
fn recipe_problems(recipe: &[Software]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in recipe {
        if !seen.insert(s.name.as_str()) {
            problems.push(format!("duplicate [software \"{}\"] stanza (the second is silently ignored)", s.name));
        }
        for b in s.builds_view() {
            if let Some((path, _)) = b.artifact {
                if escapes_root(path) {
                    let tag = match b.platform {
                        Some(p) => format!("{} [{}]", s.name, p),
                        None => s.name.clone(),
                    };
                    problems.push(format!("{tag} artifact path `{path}` escapes the worktree (absolute or `..`)"));
                }
            }
        }
        if let Some(h) = &s.hooks {
            if escapes_root(h) {
                problems.push(format!("{} hooks path `{h}` escapes the worktree (absolute or `..`)", s.name));
            }
        }
    }
    problems
}

/// The first recorded branch/name token that would escape the host root (issue #2): the
/// component name, its canonical branch, a parallel `worktrees` branch, or a `worktree =`
/// line's branch. `software --check` HAZARDs the same condition, but the fresh-clone flow
/// runs `--materialize` first and it mutates the tree, so materialize fails closed on this
/// too — reusing the one `escapes_root` check so the two paths cannot diverge. Pure (no
/// filesystem), so it is unit-tested.
fn worktree_escapes(s: &Software) -> Option<&str> {
    std::iter::once(s.name.as_str())
        .chain(std::iter::once(s.branch.as_str()))
        .chain(s.worktrees.iter().map(String::as_str))
        .chain(s.lines.iter().map(|w| w.branch.as_str()))
        .find(|t| escapes_root(t))
}

/// The Where-room software root: every component materializes under `<root>/software/`
/// (plan/0029), replacing the old root-scattered `<name>/`, `<name>.git/`, `<name>.<line>/`.
fn software_dir(root: &Path) -> PathBuf {
    root.join("software")
}

/// A component's directory: `<root>/software/<name>/`.
fn component_dir(root: &Path, name: &str) -> PathBuf {
    software_dir(root).join(name)
}

/// The final path segment of a branch ref (`feature/login` -> `login`). git keys a
/// worktree's admin entry by this leaf, so two branches sharing it collide.
fn branch_leaf(branch: &str) -> &str {
    branch.rsplit('/').next().unwrap_or(branch)
}

/// Branch names a component declares that collide once materialized (plan/0029):
/// two differing only in case collide as a path on a case-folding filesystem
/// (`/mnt/c`), and two sharing a ref leaf collide in git's worktree admin. Returns
/// one HAZARD line per colliding pair; a clean recipe yields none.
fn branch_collision_problems(s: &Software) -> Vec<String> {
    let mut branches: Vec<String> = vec![s.branch.clone()];
    branches.extend(s.worktrees.iter().cloned());
    branches.extend(s.lines.iter().map(|w| w.branch.clone()));
    let mut out = Vec::new();
    for i in 0..branches.len() {
        for j in (i + 1)..branches.len() {
            let (a, b) = (&branches[i], &branches[j]);
            if a == b {
                continue; // an exact duplicate is a separate recipe error, not a collision
            }
            if a.eq_ignore_ascii_case(b) {
                out.push(format!(
                    "HAZARD   software/{}: branches {a:?} and {b:?} collide case-insensitively (a case-folding filesystem maps them to one path)",
                    s.name
                ));
            } else if branch_leaf(a).eq_ignore_ascii_case(branch_leaf(b)) {
                out.push(format!(
                    "HAZARD   software/{}: branches {a:?} and {b:?} share the worktree-admin leaf {:?} (git keys worktree admin by the ref leaf)",
                    s.name,
                    branch_leaf(a)
                ));
            }
        }
    }
    out
}

/// A component's bare object store: `<root>/software/<name>/.bare/`, with a `.git` file beside
/// it (`gitdir: ./.bare`, see `store_gitlink`) so a git command run in the component dir resolves
/// through to the store (call/0039). A bare repo named `.git` fought git tooling, which reads a
/// `.git` directory as a working tree's repository; `.bare` names the bare store plainly and the
/// branch worktrees sit alongside.
fn store_dir(root: &Path, name: &str) -> PathBuf {
    component_dir(root, name).join(".bare")
}

/// The gitdir-link file beside the bare store (call/0039): `<root>/software/<name>/.git`, whose
/// `gitdir: ./.bare` body points git at the sibling `.bare` store. Written by `--materialize` and
/// asserted by `--check`.
fn store_gitlink(root: &Path, name: &str) -> PathBuf {
    component_dir(root, name).join(".git")
}

/// The body of the `.git` gitdir-link file (a trailing newline, as git writes it).
const STORE_GITLINK_BODY: &str = "gitdir: ./.bare\n";

/// A component worktree, keyed by branch: `<root>/software/<name>/<branch>/`. The
/// branch keeps its slashes (`feature/login` nests); the canonical worktree is the
/// component's recorded `branch` (default `main`).
fn worktree_dir(root: &Path, name: &str, branch: &str) -> PathBuf {
    component_dir(root, name).join(branch)
}

/// A worktree's display label, e.g. `software/<name>/<branch>`.
fn worktree_label(name: &str, branch: &str) -> String {
    format!("software/{name}/{branch}")
}

/// Create the in-structure handle `link` → `target` for an external-store worktree:
/// a symlink on unix, a directory junction on Windows (a junction needs no
/// privilege, unlike a Windows symlink). The handle lives under the host root so an
/// agent editing through it writes the files under test.
#[cfg(unix)]
fn make_handle(link: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn make_handle(link: &Path, target: &Path) -> std::io::Result<()> {
    let status = process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("mklink /J failed"))
    }
}

/// A parallel line's filesystem location: the external `store` if set, else the
/// in-tree `software/<name>/<branch>/`.
fn line_target(root: &Path, name: &str, w: &Worktree) -> PathBuf {
    match &w.store {
        Some(s) => PathBuf::from(s),
        None => worktree_dir(root, name, &w.branch),
    }
}

/// True when a host-gated line should be skipped on the current OS.
fn off_platform(host: &Option<String>) -> bool {
    host.as_deref().is_some_and(|h| h != std::env::consts::OS)
}

/// Add a git worktree at `path` from the bare store `bare`, creating the parent dirs
/// first (a nested branch like `feature/login` needs `…/feature/` to exist), and
/// initializing submodules on success. Returns whether the add succeeded.
fn add_worktree(bare: &Path, path: &Path, args: &[&str]) -> bool {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if !git_ok(bare, args) {
        return false;
    }
    git_ok(path, &["submodule", "update", "--init", "--recursive"]);
    true
}

/// `--materialize`: clone the bare store and add the worktrees, skipping any that
/// already exist. The bare clone needs its remote-tracking refspec set by hand —
/// `git clone --bare` does not write one — before a `fetch`/`worktree` resolves a
/// remote branch.
/// The toolchain image *reference* (registry, name and tag), never its digest:
/// `docker.io/clux/muslrust:1.95.0-stable@sha256:15a7…` -> `docker.io/clux/muslrust:1.95.0-stable`.
/// The receipt records the event's context; the digest is ambient machine state the
/// envhash owns, and recording it here would make the two artifacts overlap (plan/0074).
fn image_reference(toolchain: &str) -> &str {
    toolchain.split('@').next().unwrap_or(toolchain).trim()
}

/// The materialize receipt's evidence: event-level facts only — how much was realized for
/// which component, the pin by reference, and the toolchain image reference. Never a hook
/// binary hash, an absolute path, a submodule init state or an image digest; those are the
/// envhash's dimensions, and the two files share no fact by name (plan/0074).
fn materialize_evidence(s: &Software, items: usize) -> String {
    let mut e = format!("{items} item(s) realized for {} at pin {} (reference)", s.name, short(&s.pin));
    if let Some(t) = &s.toolchain {
        e.push_str(&format!("; toolchain {}", image_reference(t)));
    }
    e
}

/// Append one component's materialize receipt (#18): the EVENT, in the same append-only
/// shape as the `embed` receipt, under a `materialize` phase the lifecycle manifest does
/// not declare — so it records provenance without becoming a gated obligation. Advisory on
/// failure: the worktrees are already realized, and losing the note is not worth undoing
/// them.
fn append_materialize_receipt(root: &Path, s: &Software, items: usize) {
    let r = Receipt {
        phase: "materialize".to_string(),
        component: Some(s.name.clone()),
        disposition: "done".to_string(),
        evidence: Some(materialize_evidence(s, items)),
        reason: None,
        tool: Some(format!("host-lifecycle@{}", env!("CARGO_PKG_VERSION"))),
        recorded: Some(today()),
    };
    match append_receipt(root, &r) {
        Ok(()) => println!("receipt  materialize ({})", s.name),
        Err(e) => eprintln!("host-lifecycle: cannot record the materialize receipt for {}: {e}", s.name),
    }
}

fn software_materialize(root: &Path, recipe: &[Software], partial: bool) {
    let mut made = 0usize;
    for s in recipe {
        // What this component realized in THIS run: a component whose worktrees all
        // existed already is not an event, so it leaves no receipt and a re-run of a
        // materialized tree appends nothing.
        let before = made;
        // Where-room invariant (issue #2): refuse to materialize a recipe whose name, branch,
        // or any worktree branch escapes the host root. `software --check` HAZARDs the same,
        // but materialize runs first in the fresh-clone flow and clones/creates worktrees, so
        // it must fail closed before touching the filesystem.
        if let Some(esc) = worktree_escapes(s) {
            eprintln!("host-lifecycle: refusing to materialize {}: `{esc}` escapes the host root", s.name);
            process::exit(2);
        }
        // One-time migration from the plan/0029 layout (a bare repo named `.git`) to call/0039's
        // `.bare` store: rename the old `.git` bare directory to `.bare` and repair the existing
        // worktrees' gitdir links, before the clone check so no stray second `.bare` is cloned and
        // no `.git` gitdir-link is written over a directory. The recorded pin reproduces either
        // way, and the rename preserves the local store. Idempotent: once `.git` is the gitdir-link
        // file (not a directory) this is skipped.
        let old_store = component_dir(root, &s.name).join(".git");
        let new_store = store_dir(root, &s.name);
        if old_store.is_dir() {
            if new_store.exists() {
                eprintln!("host-lifecycle: software/{}: both the old `.git` bare store and a `.bare` store exist; remove the stray one and re-run (the old bare store is the `.git` directory)", s.name);
                process::exit(2);
            }
            if let Err(e) = fs::rename(&old_store, &new_store) {
                eprintln!("host-lifecycle: cannot migrate software/{}/.git to .bare: {e}", s.name);
                process::exit(2);
            }
            git_ok(&new_store, &["worktree", "repair"]);
            println!("migrate  software/{}/.git -> .bare (call/0039)", s.name);
        }
        let bare = store_dir(root, &s.name);
        let bare_rel = format!("software/{}/.bare", s.name);
        let canon = worktree_dir(root, &s.name, &s.branch);
        if bare.exists() {
            println!("skip     {bare_rel} (exists)");
        } else {
            // A partial clone (`--filter=blob:none`) lands every commit and tree but no
            // blobs, so the pin still reproduces while the whole-history blob download is
            // deferred to worktree checkout (host-lifecycle#14). Opt-in via `--partial`.
            let mut clone_args: Vec<&str> = vec!["clone", "--bare"];
            if partial {
                clone_args.push("--filter=blob:none");
            }
            clone_args.push(s.url.as_str());
            clone_args.push(bare_rel.as_str());
            if !git_ok(root, &clone_args) {
                eprintln!("host-lifecycle: git clone --bare failed for {}", s.name);
                process::exit(2);
            }
            git_ok(&bare, &["config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"]);
            git_ok(&bare, &["fetch", "origin"]);
            println!("clone    {bare_rel}");
            made += 1;
        }
        // Write the `.git` gitdir-link file beside the bare store (call/0039) so a git command
        // run in `software/<name>/` resolves through to `.bare`. Unconditional (when the store
        // exists) so a re-materialize self-heals a half-migrated component missing its link.
        if bare.is_dir() {
            let gitlink = store_gitlink(root, &s.name);
            if fs::read_to_string(&gitlink).ok().as_deref() != Some(STORE_GITLINK_BODY) {
                if let Err(e) = fs::write(&gitlink, STORE_GITLINK_BODY) {
                    eprintln!("host-lifecycle: cannot write {}: {e}", gitlink.display());
                    process::exit(2);
                }
            }
        }
        // Clear stale worktree admin (a prior teardown or move leaves entries that are
        // registered but missing on disk; plan/0029) so a re-materialize re-adds a
        // worktree path instead of hard-failing with "missing but already registered".
        if bare.is_dir() {
            git_ok(&bare, &["worktree", "prune"]);
        }
        // Canonical worktree: on its `branch`, reset to the `pin` (plan/0029) — `-B`
        // creates-or-resets the branch, so the audited tree is the pin on a real branch.
        let canon_label = worktree_label(&s.name, &s.branch);
        if canon.exists() {
            println!("skip     {canon_label} (exists)");
        } else if !add_worktree(&bare, &canon, &["worktree", "add", "-B", &s.branch, &canon.to_string_lossy(), &s.pin]) {
            eprintln!("host-lifecycle: worktree add {canon_label} @ {} failed", short(&s.pin));
            process::exit(2);
        } else {
            println!("worktree {canon_label} @ {}", short(&s.pin));
            made += 1;
        }
        // Bare `worktrees = <branch> …`: a parallel line per branch (existing branch at
        // its tip, else created at the component pin), keyed by branch under the component.
        for branch in &s.worktrees {
            let wtp = worktree_dir(root, &s.name, branch);
            let label = worktree_label(&s.name, branch);
            if wtp.exists() {
                println!("skip     {label} (exists)");
                continue;
            }
            let wtp_s = wtp.to_string_lossy().to_string();
            let exists = git_ok(&bare, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]);
            let args: Vec<&str> = if exists {
                vec!["worktree", "add", &wtp_s, branch]
            } else {
                vec!["worktree", "add", "-b", branch, &wtp_s, &s.pin]
            };
            if !add_worktree(&bare, &wtp, &args) {
                eprintln!("host-lifecycle: worktree add {label} failed");
                process::exit(2);
            }
            println!("worktree {label} ({branch})");
            made += 1;
        }
        // Explicit `worktree = <branch> <pin> …`: own branch at own pin, maybe off-tree.
        for w in &s.lines {
            let label = worktree_label(&s.name, &w.branch);
            if off_platform(&w.host) {
                println!("skip     {label} (host {}, not {})", w.host.as_deref().unwrap_or(""), std::env::consts::OS);
                continue;
            }
            let handle = worktree_dir(root, &s.name, &w.branch);
            let target = line_target(root, &s.name, w);
            let target_s = target.to_string_lossy().to_string();
            if target.exists() {
                println!("skip     {target_s} (exists)");
            } else if !add_worktree(&bare, &target, &["worktree", "add", "-B", &w.branch, &target_s, &w.pin]) {
                eprintln!("host-lifecycle: worktree add {target_s} @ {} failed", short(&w.pin));
                process::exit(2);
            } else {
                println!("worktree {target_s} ({} @ {})", w.branch, short(&w.pin));
                made += 1;
            }
            // The in-structure handle: a symlink/junction so an external store still
            // surfaces under the host root (issue #2).
            if w.store.is_some() && fs::symlink_metadata(&handle).is_err() {
                if let Some(parent) = handle.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Err(e) = make_handle(&handle, &target) {
                    eprintln!("host-lifecycle: handle {label} -> {target_s} failed: {e}");
                    process::exit(2);
                }
                println!("handle   {label} -> {target_s}");
                made += 1;
            }
        }
        if made > before {
            append_materialize_receipt(root, s, made - before);
        }
    }
    // The second writer at this call site (plan/0074): the run's event went to the
    // provenance ledger, and the tree's new state goes to the fingerprint. The two
    // share no fact, and the fingerprint is refreshed even when nothing was realized,
    // because a re-run still observes the current tree.
    envhash::write_envhash(root, recipe);
    println!("-- {made} item(s) materialized");
}

/// A reason a worktree must not be destroyed without `--force`: it holds uncommitted
/// changes, or commits not reachable from any remote ref (unpushed work). `None` when
/// the worktree is clean and fully pushed, so re-materializing from `url` + `pin`
/// loses nothing (plan/0029).
fn worktree_unsaved(wt: &Path) -> Option<String> {
    if let Some(st) = git_out(wt, &["status", "--porcelain"]) {
        if !st.is_empty() {
            return Some("has uncommitted changes".to_string());
        }
    }
    // Local-branch commits not reachable from any remote-tracking ref.
    if let Some(unpushed) = git_out(wt, &["log", "--branches", "--not", "--remotes", "--oneline"]) {
        if !unpushed.is_empty() {
            return Some("has commits not pushed to a remote".to_string());
        }
    }
    None
}

/// `--teardown [--item <name>[@<branch>]]`: remove each selected component's
/// materialized worktrees and bare store (it re-materializes from `url` + `pin`).
/// Refuses (exit 1) to destroy a worktree holding uncommitted changes or unpushed
/// commits unless `--force`, so unsaved work is never silently lost (plan/0029). A
/// component that is not materialized is skipped; the operation is idempotent.
fn software_teardown(root: &Path, recipe: &[Software], force: bool) {
    let mut removed = 0usize;
    let mut bad = 0usize;
    for s in recipe {
        let comp = component_dir(root, &s.name);
        let bare = store_dir(root, &s.name);
        if !comp.exists() {
            println!("skip     software/{} (not materialized)", s.name);
            continue;
        }
        // Every worktree path this component could have materialized (canonical,
        // bare `worktrees =` branches, and explicit `worktree =` lines incl. off-tree
        // `store=` targets).
        let mut worktrees: Vec<(String, PathBuf)> =
            vec![(worktree_label(&s.name, &s.branch), worktree_dir(root, &s.name, &s.branch))];
        for b in &s.worktrees {
            worktrees.push((worktree_label(&s.name, b), worktree_dir(root, &s.name, b)));
        }
        for w in &s.lines {
            worktrees.push((worktree_label(&s.name, &w.branch), line_target(root, &s.name, w)));
        }
        if !force {
            let mut unsafe_work = false;
            for (label, wt) in &worktrees {
                if !wt.is_dir() {
                    continue;
                }
                if let Some(reason) = worktree_unsaved(wt) {
                    println!("UNSAFE   {label} {reason}");
                    unsafe_work = true;
                }
            }
            if unsafe_work {
                println!("refuse   software/{} not torn down (commit and push, or pass --force)", s.name);
                bad += 1;
                continue;
            }
        }
        // Remove worktrees through git (it owns the off-tree `store=` target too), prune
        // the admin, then delete the in-tree component dir (store + branch dirs + handle).
        for (_, wt) in &worktrees {
            if wt.exists() {
                git_ok(&bare, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
            }
        }
        if bare.is_dir() {
            git_ok(&bare, &["worktree", "prune"]);
        }
        if let Err(e) = fs::remove_dir_all(&comp) {
            eprintln!("host-lifecycle: could not remove {}: {e}", comp.to_string_lossy());
            bad += 1;
            continue;
        }
        println!("teardown software/{}", s.name);
        removed += 1;
    }
    println!("-- {removed} component(s) torn down");
    if bad > 0 {
        process::exit(1);
    }
}

/// `--check`: each component's bare store and canonical worktree must exist, and
/// the worktree must sit at the recorded `pin`. Exit 1 if any is missing or drifted.
/// Re-check every recorded upgrade claim in `.host` against the ledger (plan/0022
/// step 6): an applied entry whose declared `depends` is unapplied, or whose `verify`
/// post-condition no longer holds, is a corrupt claim — `HAZARD`. Also surfaces the
/// partial-upgrade state for a cold auditor. Read-only (never migrates the stamp);
/// returns 0 silently when the repo carries no stamp or ledger. A nested invocation
/// (a `verify` that itself ran `software --check`) skips the verify re-check, so a
/// verify command cannot recurse infinitely.
fn upgrade_claim_problems(root: &Path) -> usize {
    let Ok(stamp) = fs::read_to_string(root.join(STAMP)) else { return 0 };
    let Some(template) = find_template_dir(root) else { return 0 };
    let Ok(text) = fs::read_to_string(template.join("UPGRADING.md")) else { return 0 };
    let entries = parse_upgrading(&text);
    if entries.is_empty() {
        return 0;
    }
    let ledger_ids: Vec<String> = entries.iter().map(|e| e.revision.clone()).collect();
    let applied = read_applied_ids(root);
    let baseline = baseline_field(&stamp)
        .or_else(|| parse_revision(&stamp).and_then(|r| derive_baseline(&template, &ledger_ids, &r)));
    let base = baseline.as_deref();
    let is_applied = |id: &str| entry_applied(id, &ledger_ids, base, &applied);
    let nested = std::env::var_os("HOST_LIFECYCLE_IN_CHECK").is_some();
    let mut bad = 0usize;
    for e in &entries {
        if !is_applied(&e.revision) {
            continue;
        }
        for d in &e.depends {
            if !is_applied(d) {
                println!("HAZARD   upgrade {} applied but its dependency {} is not", short(&e.revision), short(d));
                bad += 1;
            }
        }
        if !e.verify.is_empty() && !nested && !run_verify(root, &e.verify) {
            println!("HAZARD   upgrade {} claimed applied but its verify no longer holds: {}", short(&e.revision), e.verify);
            bad += 1;
        }
    }
    let pending = entries.iter().filter(|e| !is_applied(&e.revision)).count();
    match base {
        Some(b) => println!("ok       upgrade: baseline {}, {} applied out of order, {} pending", short(b), applied.len(), pending),
        None => println!("note     upgrade: stamp has no baseline yet (run host-lifecycle upgrade to migrate)"),
    }
    bad
}

/// Returns the count of hazards (`bad`); the caller settles the verdict and exits.
/// Returning the count rather than calling `process::exit` here makes the hazarded
/// verdict observable in a test (plan/0051 finding 3 — the off-pin HAZARD was untestable
/// while this exited in place).
/// Thin wrapper for the tests that only assert the fault count; the owed-list is
/// discarded.
#[cfg(test)]
fn software_check(root: &Path, recipe: &[Software]) -> usize {
    software_check_owed(root, recipe, &mut Vec::new())
}

/// As `software_check`, but pushes every onboarding component (declares a `deps-bundle`,
/// no committed lock) onto `owed` so the caller can surface the graduations owed
/// (plan/0057). Owed is advisory — it never adds to the returned fault count.
fn software_check_owed(root: &Path, recipe: &[Software], owed: &mut Vec<String>) -> usize {
    let mut bad = 0usize;
    for s in recipe {
        let bare = store_dir(root, &s.name);
        let canon = worktree_dir(root, &s.name, &s.branch);
        let canon_label = worktree_label(&s.name, &s.branch);
        // Where-room invariant (issue #2): every materialized worktree path must stay
        // under the host root. A component name or branch that escapes — absolute or
        // `..`-climbing — is the wrong-tree footgun; the sanctioned off-tree store is a
        // `store=` line, whose in-tree handle stays under the root.
        if escapes_root(&s.name) || escapes_root(&s.branch) {
            println!("HAZARD   {canon_label} escapes the host root");
            bad += 1;
            continue;
        }
        for branch in &s.worktrees {
            if escapes_root(branch) {
                println!("HAZARD   {} escapes the host root", worktree_label(&s.name, branch));
                bad += 1;
            }
        }
        // Branch names that collide as a path (case-folding) or in git's worktree admin
        // (shared ref leaf) are a recipe defect, detectable without materialization.
        for line in branch_collision_problems(s) {
            println!("{line}");
            bad += 1;
        }
        if !bare.is_dir() {
            println!("MISSING  software/{}/.bare (run --materialize)", s.name);
            bad += 1;
            continue;
        }
        // The `.git` gitdir-link must sit beside the store and point at `.bare` (call/0039); a
        // half-migrated component with the store but no link (or a stale link) is caught here.
        if fs::read_to_string(store_gitlink(root, &s.name)).ok().as_deref() != Some(STORE_GITLINK_BODY) {
            println!("MISSING  software/{}/.git gitdir-link -> .bare (run --materialize)", s.name);
            bad += 1;
            continue;
        }
        if !canon.is_dir() {
            println!("MISSING  {canon_label} (run --materialize)");
            bad += 1;
            continue;
        }
        let want = git_out(&bare, &["rev-parse", &s.pin]);
        let have = git_out(&canon, &["rev-parse", "HEAD"]);
        match (want, have) {
            (Some(w), Some(h)) if w == h => println!("ok       {canon_label} @ {}", short(&s.pin)),
            (Some(w), Some(h)) => {
                println!("DRIFT    {canon_label} at {} but pinned to {}", short(&h), short(&w));
                bad += 1;
            }
            _ => {
                println!("ERROR    {canon_label} — cannot resolve HEAD or pin");
                bad += 1;
            }
        }
        // Explicit parallel worktrees: each at its own branch and pin (issue #6),
        // optionally backed by an external store reached through an in-tree handle
        // (issue #2). The pin/branch check runs against the resolved store.
        for w in &s.lines {
            let label = worktree_label(&s.name, &w.branch);
            if off_platform(&w.host) {
                println!("skip     {label} (host {}, not {})", w.host.as_deref().unwrap_or(""), std::env::consts::OS);
                continue;
            }
            if escapes_root(&w.branch) {
                println!("HAZARD   {label} escapes the host root (use store=<path> with an in-tree handle)");
                bad += 1;
                continue;
            }
            let handle = worktree_dir(root, &s.name, &w.branch);
            let wt = line_target(root, &s.name, w);
            // A store-backed line: the in-tree handle must resolve to the store.
            if w.store.is_some() {
                match (fs::canonicalize(&handle), fs::canonicalize(&wt)) {
                    (Ok(h), Ok(t)) if h == t => {}
                    (Ok(h), Ok(t)) => {
                        println!("HAZARD   {label} resolves to {} not the store {}", h.display(), t.display());
                        bad += 1;
                        continue;
                    }
                    _ => {
                        println!("HAZARD   {label} has no in-structure handle to store {} (run --materialize)", wt.to_string_lossy());
                        bad += 1;
                        continue;
                    }
                }
            }
            if !wt.is_dir() {
                println!("MISSING  {label} (run --materialize)");
                bad += 1;
                continue;
            }
            let want = git_out(&bare, &["rev-parse", &w.pin]);
            let have = git_out(&wt, &["rev-parse", "HEAD"]);
            let br = git_out(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
            match (want, have) {
                (Some(want), Some(have)) if want == have => match br {
                    Some(br) if br == w.branch => println!("ok       {label} ({} @ {})", w.branch, short(&w.pin)),
                    Some(br) => {
                        println!("DRIFT    {label} at {} but on branch {} not {}", short(&w.pin), br, w.branch);
                        bad += 1;
                    }
                    None => {
                        println!("ok       {label} @ {}", short(&w.pin));
                    }
                },
                (Some(want), Some(have)) => {
                    println!("DRIFT    {label} at {} but pinned to {}", short(&have), short(&want));
                    bad += 1;
                }
                _ => {
                    println!("ERROR    {label} — cannot resolve HEAD or pin");
                    bad += 1;
                }
            }
        }
        // Reproducible-build provenance: deploy line recorded, exemption cited,
        // deployed artifact attested (issue #10).
        bad += provenance_problems_owed(root, s, owed);
        // Verification lanes are mandatory when a spec of their kind exists: a
        // materialized component carrying a `.allium`/`.tla` must run its lane.
        bad += spec_lane_problems(root, s);
        // A declared deeper rung must have a RUNNABLE re-deriver, not merely a present CI
        // lane (call/0018, plan/0048): re-derivation that cannot run is not discharged.
        bad += tier_rederiver_problems(root, s);
    }
    // Worktree-absence coherence (call/0005): a tracked symlink whose target is not
    // itself tracked here points into a separately-materialized path — a software
    // worktree or a tool submodule — so it dangles wherever that path is not
    // materialized (a fresh clone, CI, a partial submodule init).
    for (link, target) in dangling_symlink_hazards(root) {
        println!("HAZARD   {link} -> {target} (symlink into an un-materialized path; not tracked here)");
        bad += 1;
    }
    // Generated (untracked) skill links that dangle (plan/0029): the tracked-symlink
    // hazard above cannot see them, yet a dangling one trips a tree-walker (the Site-CI
    // regression). Re-run link-skills.sh after (de)materialization to clear it.
    for link in dangling_generated_links(root) {
        println!("HAZARD   .claude/skills/{link} dangles (run link-skills.sh after materialize)");
        bad += 1;
    }
    // Re-check every recorded upgrade claim against the ledger (plan/0022 step 6).
    bad += upgrade_claim_problems(root);
    // #12: a spec under plan/*/spec/ evades the mandatory lanes — co-locate it with software.
    bad += plan_spec_problems(root);
    // plan/0025: every manifest phase emits a receipt; re-check each `done` by the
    // manifest's closed `recheck =`. Inert until the adopted template declares a manifest.
    bad += receipt_gate_problems(root, recipe);
    // plan/0042: the receipted task graph — structural problems (parse, graph, reference)
    // and the per-task receipt gate. Inert until a plan adopts the anchored-task form.
    bad += task_check_problems(root);
    // call/0038: the host-template's prose CI must pin the same host-lifecycle commit the host
    // gates on; a drifted pin ships new adopters a stale tool. Inert unless this repo develops
    // host-lifecycle and carries the template submodule, so it fires only on the dev host.
    bad += template_pin_problems(root, recipe);
    bad
}

/// The receipt gate over `software --check` (plan/0025): load the manifest the
/// project is governed by, re-check each phase's receipt, and execute the closed
/// `recheck =` for every `done`. Inert when the adopted template has no manifest —
/// unless receipts already exist with no manifest to re-check them (R4 HAZARD).
fn receipt_gate_problems(root: &Path, recipe: &[Software]) -> usize {
    let phases = match load_project_manifest(root) {
        ManifestState::NotAdopted => return 0,
        ManifestState::Live(ps) => ps,
        ManifestState::Absent => {
            let has_receipts = !read_all_receipts(root).is_empty();
            if has_receipts {
                println!("HAZARD   {RECEIPTS} present but the adopted template has no {MANIFEST} to re-check them");
                return 1;
            }
            return 0;
        }
    };
    let receipts = read_all_receipts(root);
    let components: Vec<String> = recipe.iter().map(|s| s.name.clone()).collect();
    let mut bad = 0;
    for line in receipt_gate(&phases, &receipts, !recipe.is_empty(), &components) {
        let recheck_failed = line.recheck.as_deref().is_some_and(|cmd| !run_recheck(root, cmd));
        if line.ok && !recheck_failed {
            println!("ok       {} — {}", line.label, line.note);
            continue;
        }
        let note = if recheck_failed { format!("{} — recheck FAILED, re-opened", line.note) } else { line.note };
        println!("HAZARD   {} — {note}", line.label);
        if let Some(rem) = &line.remedy {
            println!("           remedy: {rem}");
        }
        bad += 1;
    }
    bad
}

/// Run a receipt's closed `recheck =` command in the project root; a non-zero exit
/// re-opens the `done` (R1 — evidence is re-derived, never self-asserted).
fn run_recheck(root: &Path, cmd: &str) -> bool {
    process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Generated (untracked) skill links under `.claude/skills/` that dangle (plan/0029):
/// a symlink whose target no longer resolves. These are deliberately untracked
/// (`call/0005`), so the tracked-symlink hazard cannot see them; a dangling one trips
/// any tree-walker (mdBook), the regression that reddened Site CI. Returns the dangling
/// link names; an absent `.claude/skills/` (nothing generated yet) yields none.
fn dangling_generated_links(root: &Path) -> Vec<String> {
    let skills = root.join(".claude").join("skills");
    let mut bad = Vec::new();
    let Ok(rd) = fs::read_dir(&skills) else {
        return bad;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        let is_link = fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        // a symlink (symlink_metadata Ok+symlink) whose target is gone (metadata Err)
        if is_link && fs::metadata(&p).is_err() {
            bad.push(e.file_name().to_string_lossy().to_string());
        }
    }
    bad.sort();
    bad
}

/// Tracked symlinks whose resolved target is **not itself tracked here** — they
/// point into a separately-materialized path (a software worktree, or a sub-path of
/// a tool submodule) and dangle wherever it is not materialized (`call/0005`).
/// `(link, resolved)` pairs. A symlink to the submodule root (a tracked gitlink) is
/// not flagged: it resolves to the empty dir git leaves on checkout.
fn dangling_symlink_hazards(root: &Path) -> Vec<(String, String)> {
    let out = match process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-s"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out);
    // One pass: every tracked path (blobs + gitlinks), and which are symlinks.
    let mut tracked: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut links: Vec<&str> = Vec::new();
    for line in text.lines() {
        // "<mode> <hash> <stage>\t<path>"; mode 120000 marks a symlink.
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        tracked.insert(path);
        if meta.starts_with("120000") {
            links.push(path);
        }
    }
    let mut hazards = Vec::new();
    for link in links {
        let Ok(target) = fs::read_link(root.join(link)) else {
            continue;
        };
        let target = target.to_string_lossy().replace('\\', "/");
        let parent = link.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let resolved = normalize_join(parent, &target);
        if !tracked.contains(resolved.as_str()) {
            hazards.push((link.to_string(), resolved));
        }
    }
    hazards
}

/// Join a base dir and a relative target, resolving `.`/`..` lexically.
fn normalize_join(base: &str, rel: &str) -> String {
    let mut comps: Vec<&str> = Vec::new();
    for part in base.split('/').chain(rel.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                comps.pop();
            }
            p => comps.push(p),
        }
    }
    comps.join("/")
}

/// `git -C <cwd> <args>` for side effects; true on success, output suppressed.
fn git_ok(cwd: &Path, args: &[&str]) -> bool {
    process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `git -C <cwd> <args>` capturing trimmed stdout, or `None` on failure.
fn git_out(cwd: &Path, args: &[&str]) -> Option<String> {
    let o = process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// First 12 chars of a SHA for display (ASCII hex, so byte-slicing is safe).
fn short(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

/// sha256 of a file via `sha256sum`, or None if it can't be computed.
fn sha256_file(path: &Path) -> Option<String> {
    let o = process::Command::new("sha256sum").arg(path).output().ok()?;
    o.status.success().then(|| {
        String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap_or("").to_string()
    })
}

/// Does the `call/NNNN[-slug]` decision a `repro-exempt` citation names exist?
fn cited_decision_exists(root: &Path, cite: &str) -> bool {
    let num = cite.trim_start_matches("call/").split('-').next().unwrap_or("");
    if num.is_empty() {
        return false;
    }
    let pfx = format!("{num}-");
    fs::read_dir(root.join("call")).ok().is_some_and(|rd| {
        rd.filter_map(|e| e.ok()).any(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with(&pfx) && n.ends_with(".md")
        })
    })
}

/// Attestation + exemption checks for a component's build provenance (the cheap pass;
/// the rebuild *proof* is `--verify-build`). Returns the count of failures.
/// Thin wrapper for the tests that only assert the fault count; the onboarding owed-list
/// is discarded.
#[cfg(test)]
fn provenance_problems(root: &Path, s: &Software) -> usize {
    provenance_problems_owed(root, s, &mut Vec::new())
}

/// Reproducible-build provenance for one component. Returns the fault count (`bad`). A
/// component that declares a `deps-bundle` but has not yet committed its lock is an
/// onboarding component: not a fault (it stays green), but its name is pushed onto
/// `owed` so `software --check` can surface the graduation it owes (plan/0057).
fn provenance_problems_owed(root: &Path, s: &Software, owed: &mut Vec<String>) -> usize {
    let mut bad = 0;
    let host = std::env::consts::OS;
    for b in s.builds_view() {
        // Tag findings with the platform when there is more than the default build.
        let tag = match b.platform {
            Some(p) => format!("{} [{}]", s.name, p),
            None => s.name.clone(),
        };
        // The deployed line must be a recorded worktree (canonical or an explicit
        // line) — a static check, independent of the build host.
        if let Some(dep) = b.deploy {
            if dep == s.name || dep == s.branch || s.lines.iter().any(|w| w.branch == dep) {
                println!("ok       {tag} deploy line `{dep}` is recorded");
            } else {
                println!("DRIFT    {tag} deploy line `{dep}` is not a recorded worktree");
                bad += 1;
            }
        }
        // A build attests only on its `attest-host`; on any other host it is
        // skipped (its artifact is not built here), as an exempt build is skipped.
        if let Some(ah) = b.attest_host {
            if ah != host {
                println!("skip     {tag} attested on {ah} (host is {host})");
                continue;
            }
        }
        // An exemption must cite a real decision.
        if let Some(cite) = b.repro_exempt {
            if cited_decision_exists(root, cite) {
                println!("warn     {tag} build is repro-exempt ({cite}) — reproducibility not proven");
            } else {
                println!("DRIFT    {tag} repro-exempt cites missing decision {cite}");
                bad += 1;
            }
        }
        // host#14 stricter minimum: an `artifact` with no `toolchain` pin cannot be
        // reproducibly verified — `--verify-build` has no recorded environment to
        // rebuild in. An exempt build is excused (it cites a decision above).
        if b.artifact.is_some() && b.toolchain.is_none() && b.repro_exempt.is_none() {
            println!("HAZARD   {tag} records an artifact but no `toolchain` pin — not reproducibly verifiable");
            bad += 1;
        }
        // Attestation: when the artifact is present in the canonical worktree, a
        // match is a positive "verified" note. A mismatch is *not* a fault here:
        // the recorded hash is the pinned container's output, and a local toolchain
        // legitimately differs (the same reasoning `--install-hooks` uses). The
        // worktree-at-pin gate is enforced by `software_check` above, and the
        // reproducibility *proof* is `--verify-build`, not this cheap pass.
        if let Some((path, sha)) = b.artifact {
            let p = worktree_dir(root, &s.name, &s.branch).join(path);
            if !p.exists() {
                println!("skip     {tag} artifact {path} not present (not a deploy/build host)");
            } else if sha256_file(&p).as_deref() == Some(sha.as_str()) {
                println!("ok       {tag} artifact {path} @ {} (verified)", short(sha));
            } else {
                println!("note     {tag} artifact {path} is a local build (differs from canonical) — proven by --verify-build");
            }
        }
    }
    // plan/0032 hermeticity gate: a component pinning a dependency bundle keeps a
    // committed `deps-bundle.lock` in its worktree as the single source of truth; the
    // recorded `.host-software` pin MUST equal it, or the producer and the orchestration
    // have drifted. The build-offline-under-`--network none` proof is `--verify-build`.
    if let Some((url, sha)) = &s.deps_bundle {
        let worktree = worktree_dir(root, &s.name, &s.branch);
        let lock = worktree.join("deps-bundle.lock");
        match fs::read_to_string(&lock) {
            Ok(text) => {
                let f: Vec<&str> = text.split_whitespace().collect();
                if f.len() >= 2 && f[0] == url && f[1] == sha {
                    println!("ok       {} deps-bundle pin matches deps-bundle.lock @ {}", s.name, short(sha));
                } else {
                    println!("HAZARD   {} deps-bundle pin differs from deps-bundle.lock (producer drift)", s.name);
                    bad += 1;
                }
            }
            // A lock git tracks but that is missing from the worktree was deleted, and the pin
            // cross-check it would run is silently bypassed while the pin still reads clean
            // (issue #6, engineered): HAZARD. A lock git never tracked is a not-yet-locked
            // onboarding component (it declares a bundle but has not committed its lock yet): a
            // lenient note, not a fault, so onboarding does not turn `--check` red.
            Err(_) => {
                if git_ok(&worktree, &["ls-files", "--error-unmatch", "deps-bundle.lock"]) {
                    println!("HAZARD   {} deps-bundle.lock is tracked but missing from the worktree — the pin cross-check is bypassed", s.name);
                    bad += 1;
                } else {
                    println!("note     {} deps-bundle pinned; deps-bundle.lock not yet committed (onboarding)", s.name);
                    owed.push(s.name.clone());
                }
            }
        }
    }
    bad
}

/// The verification lanes are mandatory **when a spec of their kind exists**: a
/// materialized component carrying any `.allium` MUST have a CI workflow running
/// `allium check` + `allium analyse`; any `.tla` MUST have a TLC lane. Returns the
/// count of components with a present spec but a missing lane (a HAZARD). An
/// un-materialized worktree is skipped (the specs cannot be seen).
fn spec_lane_problems(root: &Path, s: &Software) -> usize {
    let worktree = worktree_dir(root, &s.name, &s.branch);
    if !worktree.is_dir() {
        return 0;
    }
    let (has_allium, has_tla, has_obligations) = find_specs(&worktree);
    if !has_allium && !has_tla && !has_obligations {
        return 0;
    }
    let workflows = read_workflows(&worktree);
    let mut bad = 0;
    if has_allium {
        if workflows.contains("allium check") && workflows.contains("allium analyse") {
            println!("ok       {} allium lane present (check + analyse)", s.name);
        } else {
            println!(
                "HAZARD   {} carries a .allium spec but no CI workflow runs `allium check` + `allium analyse`",
                s.name
            );
            bad += 1;
        }
        // The obligations must be dispositioned: a `.obligations` manifest beside
        // the spec, checked by `host-lifecycle obligations` in CI.
        if has_obligations {
            println!("ok       {} obligations manifest present", s.name);
        } else {
            println!(
                "HAZARD   {} carries a .allium spec but no `.obligations` manifest (run `host-lifecycle obligations`)",
                s.name
            );
            bad += 1;
        }
    }
    if has_tla {
        if workflows.contains("tlc2.TLC") || workflows.contains("tla2tools") {
            println!("ok       {} specula/TLC lane present", s.name);
        } else {
            println!("HAZARD   {} carries a .tla spec but no CI workflow model-checks it (TLC)", s.name);
            bad += 1;
        }
    }
    // Deep-verification tiers (host-prove) are opt-in and inert: a tier's lane is
    // required only once a `.obligations` manifest *declares* it (a `kani:` /
    // `apalache:` / `tlaps:` disposition). No declaration → no requirement, no HAZARD
    // — bare `.tla`/crate presence never activates a tier.
    if has_obligations {
        // Parse the dispositions (issue #13), not a raw substring: a tier is required only
        // once a disposition genuinely `parse_rung`s to it, matching the obligations engine.
        let declared = declared_rung_tokens(&worktree);
        for (token, label, lane_present) in [
            ("kani:", "Kani code-conformance", workflows.contains("cargo kani") || workflows.contains("kani-verifier")),
            ("apalache:", "Apalache symbolic", workflows.contains("apalache-mc")),
            ("tlaps:", "TLAPS proof", workflows.contains("tlapm")),
        ] {
            if declared.contains(&token) {
                if lane_present {
                    println!("ok       {} {label} lane present (declares {token})", s.name);
                } else {
                    println!(
                        "HAZARD   {} declares an obligation {token} but no CI workflow runs the {label} lane",
                        s.name
                    );
                    bad += 1;
                }
            }
        }
    }
    bad
}

/// Every regular file under `root`, walked symlink-safely and depth-bounded. A
/// symlink is treated as a leaf and never followed, so a symlink cycle in
/// gitignored scratch (a Wine prefix, `node_modules`, a venv) cannot make the walk
/// hang (host-lifecycle#15, which wedged `software --check` in uninterruptible I/O
/// on a 9P mount); the depth cap is defence-in-depth against a non-symlink cycle (a
/// bind mount). A directory named in `skip` is pruned, and an unreadable directory
/// is skipped rather than fatal. Every recursive worktree lane shares this so none
/// re-introduces the unguarded walk. Symlink handling matches `collect_files`.
fn walk_files_safe(root: &Path, skip: &[&str]) -> Vec<PathBuf> {
    const MAX_DEPTH: usize = 256;
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue; // a symlink is a leaf; never followed (breaks a cycle, #15)
            }
            if ft.is_dir() {
                if skip.iter().any(|s| *s == e.file_name().to_string_lossy()) {
                    continue;
                }
                if depth < MAX_DEPTH {
                    stack.push((e.path(), depth + 1));
                }
            } else {
                out.push(e.path());
            }
        }
    }
    out
}

/// Concatenate every `.obligations` manifest in the worktree (for tier-declaration
/// substring checks). Skips `.git`, `target`, `node_modules`.
fn read_obligations_text(dir: &Path) -> String {
    let mut text = String::new();
    for p in walk_files_safe(dir, &[".git", "target", "node_modules"]) {
        if p.extension().and_then(|x| x.to_str()) == Some("obligations") {
            if let Ok(t) = fs::read_to_string(&p) {
                text.push_str(&t);
                text.push('\n');
            }
        }
    }
    text
}

/// The deeper rungs a worktree's `.obligations` manifests actually DECLARE — parsed with the
/// same `parse_rung` the obligations engine uses, not a raw substring over concatenated bodies
/// (issue #13): a `kani:` in a comment, or a non-rung disposition mentioning the word, must not
/// activate a tier and demand a CI lane the engine never treats as a rung. Each element is the
/// bare tool token (`kani:` / `apalache:` / `tlaps:`) the lane-name lookups key on.
fn declared_rung_tokens(dir: &Path) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    for (_id, disp) in parse_obligation_manifest(&read_obligations_text(dir)) {
        let Some(rung) = parse_rung(&disp) else { continue };
        let tok = match rung.tool.as_str() {
            "kani" => "kani:",
            "apalache" => "apalache:",
            "tlaps" => "tlaps:",
            _ => continue,
        };
        if !found.contains(&tok) {
            found.push(tok);
        }
    }
    found
}

/// Probe (cheaply, never running the proof) that a declared rung's re-deriver EXECUTES — host-prove,
/// the shared driver for every rung — not merely resolves on PATH (`call/0018`, plan/0048). The
/// rung-specific verifier (`cargo kani` / `apalache-mc` / `tlapm`) can be CI-only by design (TLAPS is,
/// the apalache JVM is optional locally), so its presence is the pluggable re-derivation's concern,
/// not this cheap, context-independent gate's: host-prove is the one driver without which no rung
/// re-derives anywhere. A declared rung whose driver cannot run is *available, not discharged* — the
/// gap that hid a missing host-prove install for two weeks. Returns `Some(reason)` when not runnable,
/// `None` when it runs (or the token is not a deeper rung, so there is nothing to probe).
fn rung_rederiver_problem(token: &str) -> Option<String> {
    if !matches!(token, "kani:" | "apalache:" | "tlaps:") {
        return None;
    }
    // host-prove must spawn to completion (execute), not merely be findable on PATH.
    if process::Command::new("host-prove").arg("--help").output().is_err() {
        Some("host-prove, the re-deriver for every rung, does not run — install it on PATH".to_string())
    } else {
        None
    }
}

/// `software --check`'s runnability pass (plan/0048): a component that declares any deeper rung must
/// have a runnable re-deriver, so the re-derivation that earns its digest can actually run. host-prove
/// is the shared driver, so one probe covers every rung the component declares. Kept out of
/// `spec_lane_problems` so the lane-present logic stays unit-tested without host-prove on PATH; this
/// pass is impure (it spawns) and is exercised by the real `software --check`. Returns the HAZARD count.
fn tier_rederiver_problems(root: &Path, s: &Software) -> usize {
    let worktree = worktree_dir(root, &s.name, &s.branch);
    if !worktree.is_dir() {
        return 0;
    }
    // Parse the dispositions (issue #13), not a raw substring over concatenated bodies.
    let declared = declared_rung_tokens(&worktree);
    if declared.is_empty() {
        return 0;
    }
    match rung_rederiver_problem(declared[0]) {
        None => {
            println!("ok       {} re-deriver runnable (declares {})", s.name, declared.join(" "));
            0
        }
        Some(reason) => {
            println!("HAZARD   {} declares {} but the re-deriver is not runnable — {reason}", s.name, declared.join(" "));
            1
        }
    }
}

/// Walk a worktree (skipping `.git`, `target`, `node_modules`) and report whether
/// any `.allium` spec, `.tla` spec, and `.obligations` manifest exist.
fn find_specs(dir: &Path) -> (bool, bool, bool) {
    let mut allium = false;
    let mut tla = false;
    let mut obligations = false;
    for p in walk_files_safe(dir, &[".git", "target", "node_modules"]) {
        match p.extension().and_then(|x| x.to_str()) {
            Some("allium") => allium = true,
            Some("tla") => tla = true,
            Some("obligations") => obligations = true,
            _ => {}
        }
    }
    (allium, tla, obligations)
}

/// Concatenate every workflow under `.github/workflows/` (`*.yml`/`*.yaml`).
fn read_workflows(worktree: &Path) -> String {
    let dir = worktree.join(".github/workflows");
    let Ok(rd) = fs::read_dir(&dir) else { return String::new() };
    let mut text = String::new();
    for e in rd.flatten() {
        let p = e.path();
        if matches!(p.extension().and_then(|x| x.to_str()), Some("yml") | Some("yaml")) {
            if let Ok(t) = fs::read_to_string(&p) {
                text.push_str(&t);
                text.push('\n');
            }
        }
    }
    text
}

/// `obligations <spec.allium> [--manifest <f>] [--tests <dir>] [--prove <dir>]` —
/// the remap-dictionary discipline for tests: every obligation `allium plan` derives
/// from the spec MUST be dispositioned in the sibling `<stem>.obligations`
/// manifest. Each line is `<id> => <disposition>`, where the disposition is
/// `test:<name>` (a named test discharges it), `structural` (the spec's own
/// `check`/`analyse` lane covers it), `waived: <reason>`, or a deep-verification
/// tier — `kani:<harness>`, `apalache:<inv>`, `tlaps:<theorem>` (host-prove). Fails
/// on any obligation with no disposition, any stale manifest id no longer derived,
/// any `test:<name>` absent from the `--tests` sources, and any tier proof name
/// absent from the `--prove` sources (the crate / `.tla`).
/// Run `allium plan <spec>` and return its stdout, or an error message. Shared by the
/// `obligations` CLI and the release-gate discharge (plan/0069).
fn allium_plan(spec: &Path) -> Result<String, String> {
    match process::Command::new("allium").arg("plan").arg(spec).output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => Err(format!("`allium plan` failed: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => Err(format!("cannot run `allium plan` (is allium-cli installed?): {e}")),
    }
}

/// The `.allium` spec paths under `dir` (symlink-safe; skips build/cache dirs), for the
/// release gate (plan/0069) to find the released component's specs.
fn find_allium_specs(dir: &Path) -> Vec<PathBuf> {
    walk_files_safe(dir, &[".git", "target", "node_modules"])
        .into_iter()
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("allium"))
        .collect()
}

/// The STALE-input-digest problems for one obligations manifest: the release gate's
/// allium-free arm (plan/0069). Reads the dispositions and the sibling `.digests` ledger
/// and reports any rung whose declared inputs drifted since the recorded re-derivation
/// (the born-red-tag class). Empty means clean.
fn manifest_staleness_problems(manifest: &Path) -> Vec<String> {
    let dispositions = match fs::read_to_string(manifest) {
        Ok(t) => parse_obligation_manifest(&t),
        Err(_) => return Vec::new(),
    };
    let ledger = digest_ledger_path(manifest);
    let base = manifest.parent().unwrap_or(Path::new("."));
    staleness_problems(&dispositions, base, &ledger)
}

/// The offline discharge problems for one spec, run by the release gate (plan/0069):
/// MISSING/STALE dispositions (`allium plan` + `obligation_gaps`) and STALE input
/// digests (`manifest_staleness_problems`). Empty is clean. No `--tests` (test-name
/// resolution stays in component CI) and no `--rederive` (proof re-derivation stays in
/// component CI, where the heavy verifiers live).
fn discharge_problems(spec: &Path, manifest: &Path) -> Result<Vec<String>, String> {
    let plan = allium_plan(spec)?;
    let plan_ids = extract_obligation_ids(&plan);
    if plan_ids.is_empty() {
        return Err(format!("`allium plan` produced no obligations for {}", spec.display()));
    }
    let dispositions = match fs::read_to_string(manifest) {
        Ok(t) => parse_obligation_manifest(&t),
        Err(e) => return Err(format!("cannot read manifest {} ({e}); a .allium needs a sibling .obligations", manifest.display())),
    };
    let mut problems = obligation_gaps(&plan_ids, &dispositions, None, None);
    problems.extend(manifest_staleness_problems(manifest));
    Ok(problems)
}

fn obligations(args: &[String]) {
    let mut pos: Vec<&String> = Vec::new();
    let mut manifest_arg: Option<&String> = None;
    let mut tests_arg: Option<&String> = None;
    let mut prove_arg: Option<&String> = None;
    let mut rederive_arg: Option<&String> = None;
    let mut record_digests_flag = false;
    let mut strict_discharge = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                manifest_arg = args.get(i + 1);
                i += 1;
            }
            "--tests" => {
                tests_arg = args.get(i + 1);
                i += 1;
            }
            "--prove" => {
                prove_arg = args.get(i + 1);
                i += 1;
            }
            "--rederive" => {
                rederive_arg = args.get(i + 1);
                i += 1;
            }
            "--record-digests" => record_digests_flag = true,
            "--strict-discharge" => strict_discharge = true,
            _ => pos.push(&args[i]),
        }
        i += 1;
    }
    let Some(spec) = pos.first() else {
        eprintln!("host-lifecycle obligations <spec.allium> [--manifest <file>] [--tests <dir>] [--prove <dir>] [--rederive <dir>] [--record-digests] [--strict-discharge]");
        process::exit(2);
    };
    if record_digests_flag && rederive_arg.is_none() {
        eprintln!("host-lifecycle: --record-digests requires --rederive (only a passing proof is recorded)");
        process::exit(2);
    }
    let spec_path = Path::new(spec.as_str());
    let manifest = match manifest_arg {
        Some(m) => PathBuf::from(m.as_str()),
        None => spec_path.with_extension("obligations"),
    };
    // 1. Derive the obligations from the spec.
    let plan = match allium_plan(spec_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("host-lifecycle: {e}");
            process::exit(2);
        }
    };
    let plan_ids = extract_obligation_ids(&plan);
    if plan_ids.is_empty() {
        eprintln!("host-lifecycle: `allium plan` produced no obligations for {spec}");
        process::exit(2);
    }
    // 2. Read the dispositions.
    let dispositions = match fs::read_to_string(&manifest) {
        Ok(t) => parse_obligation_manifest(&t),
        Err(e) => {
            eprintln!("host-lifecycle: cannot read manifest {} ({e}); a .allium needs a sibling .obligations", manifest.display());
            process::exit(2);
        }
    };
    let tests = tests_arg.map(|d| read_dir_recursive(Path::new(d.as_str())));
    let prove = prove_arg.map(|d| read_dir_recursive(Path::new(d.as_str())));
    // 3. Every obligation dispositioned; no stale dispositions; test/proof refs present (a
    //    presence *lint*, not discharge — #8). With `--rederive`, each rung's verifier is
    //    re-run via host-prove and must PASS at its bound (the real discharge, `call/0018`).
    let mut problems = obligation_gaps(&plan_ids, &dispositions, tests.as_deref(), prove.as_deref());
    let ledger = digest_ledger_path(&manifest);
    if let Some(d) = rederive_arg {
        let rederive_dir = Path::new(d.as_str());
        let rd = rederive_problems(&dispositions, rederive_dir);
        // 4. With --record-digests, fingerprint each rung's inputs — but only after a
        //    clean re-derivation, so the recorded digest always tracks a passing proof.
        if record_digests_flag {
            if rd.is_empty() {
                match record_digests(&dispositions, rederive_dir, &ledger) {
                    Ok(n) => println!("recorded {n} input digest(s) -> {}", ledger.display()),
                    Err(e) => problems.push(format!("DIGEST   {e}")),
                }
            } else {
                problems.push("DIGEST   --record-digests skipped: not every rung re-derived".to_string());
            }
        }
        problems.extend(rd);
    } else {
        // Offline: the cheap input-digest staleness signal (no verifier needed).
        let base = manifest.parent().unwrap_or(Path::new("."));
        problems.extend(staleness_problems(&dispositions, base, &ledger));
    }
    // plan/0052 (no-hollow-green): the strengthened `test:` discharge link. Advisory
    // by default so a tool bump never reds an adopter's green ladder; HAZARD under
    // `--strict-discharge`, which host-lifecycle's own CI runs to keep the dogfood honest.
    let discharge_warns = discharge_warnings(&plan_ids, &dispositions, tests.as_deref());
    if strict_discharge {
        problems.extend(discharge_warns);
    } else {
        for w in &discharge_warns {
            println!("warning: {w}  (advisory; HAZARD under --strict-discharge)");
        }
    }
    for p in &problems {
        println!("{p}");
    }
    if !problems.is_empty() {
        eprintln!("-- {} obligation(s) undispositioned, stale, missing a test, unlinked, or UNPROVEN ({})", problems.len(), manifest.display());
        process::exit(1);
    }
    let mode = if rederive_arg.is_some() { "dispositioned; rungs re-derived" } else { "dispositioned" };
    println!("-- all {} obligation(s) {mode}", plan_ids.len());
}

/// How a `test:<name>` disposition resolves against the concatenated test source.
enum TestRef<'a> {
    /// No `fn <name>(` definition (a substring of the name may still occur).
    Absent,
    /// More than one `fn <name>(` definition — the disposition is ambiguous.
    Ambiguous(usize),
    /// Exactly one definition; `body` is its brace-matched body and `ignored` is true
    /// when an `#[ignore]` attribute sits on it (an ignored test never runs under
    /// `cargo test`, so it discharges nothing).
    Found { body: &'a str, ignored: bool },
}

/// Resolve `test:<name>` to its definition in `src` by an exact `fn <name>(` match,
/// not a substring. A name that only occurs inside another identifier no longer
/// counts (the plan/0051 hole: `host_root_escape_is_detected` substring-matched while
/// driving nothing), and two definitions are `Ambiguous`. The body is brace-matched
/// from the first `{` after the signature; brace counting is a static heuristic and
/// can over-read a body that embeds `{`/`}` in a string, which is acceptable for the
/// `exercises=` containment check below.
fn resolve_test<'a>(src: &'a str, name: &str) -> TestRef<'a> {
    let needle = format!("fn {name}(");
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(rel) = src[from..].find(&needle) {
        let at = from + rel;
        let boundary = src[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if boundary {
            hits.push(at);
        }
        from = at + needle.len();
    }
    match hits.len() {
        0 => TestRef::Absent,
        1 => {
            // `#[ignore]` sits on one of the few attribute lines just above the `fn`.
            let ignored = src[..hits[0]].lines().rev().take(5).any(|l| l.contains("#[ignore"));
            match src[hits[0]..].find('{') {
                None => TestRef::Found { body: "", ignored },
                Some(rel) => {
                    let bopen = hits[0] + rel;
                    let mut depth = 0i32;
                    let mut end = src.len();
                    for (k, &b) in src.as_bytes()[bopen..].iter().enumerate() {
                        if b == b'{' {
                            depth += 1;
                        } else if b == b'}' {
                            depth -= 1;
                            if depth == 0 {
                                end = bopen + k + 1;
                                break;
                            }
                        }
                    }
                    TestRef::Found { body: &src[bopen..end], ignored }
                }
            }
        }
        n => TestRef::Ambiguous(n),
    }
}

/// The obligation kinds the spec's own `allium check`/`analyse` lane discharges
/// (field presence, enum comparability, surface shape, declared transition graph),
/// taken from the obligation id prefix. A `structural` disposition is legitimate only
/// for one of these; on a behavioural kind it is the relabel-dodge below.
fn is_behavioural_kind(id: &str) -> bool {
    let kind = id.split('.').next().unwrap_or("");
    matches!(kind, "rule-success" | "rule-failure" | "rule-entity-creation" | "invariant" | "transition-edge")
}

/// The strengthened-discharge WARNINGS (plan/0052, no-hollow-green). These are
/// ADVISORY by default and escalate to HAZARD under `--strict-discharge`, so the
/// tightening of a shared gate reaches adopters warn-then-retire rather than turning a
/// green ladder red on a tool bump. The existing `obligation_gaps` substring HAZARD is
/// left intact beneath this.
///
/// For a `test:<name> [exercises=<tok>,<tok>]` disposition, with test sources:
/// `AMBIGUOUS` (the name matches more than one definition), `HOLLOW` (an `exercises=`
/// token the discharging test must reference is absent from its body, the bite that
/// catches a test pointed at the wrong code), or `UNLINKED` (no `exercises=` declared
/// yet, the staged requirement). Two escapes are also closed, so hardening `test:` does
/// not just move the hole (the weak-agent relabel route): `RELABEL` (a behavioural
/// obligation dispositioned `structural`, which the analyse lane does not discharge) and
/// `UNWAIVED` (a `waived:` with no reason). Whether `waived:` needs a recorded-decision
/// citation is a probe question — a software repo has no `call/` to cite. Pure, so it is
/// unit-tested.
fn discharge_warnings(plan_ids: &[String], dispositions: &[(String, String)], tests: Option<&str>) -> Vec<String> {
    let mut warns = Vec::new();
    for id in plan_ids {
        let Some((_, disp)) = dispositions.iter().find(|(k, _)| k == id) else { continue };
        if let Some(rest) = disp.strip_prefix("test:") {
            let Some(src) = tests else { continue };
            let mut toks = rest.split_whitespace();
            let Some(name) = toks.next().filter(|n| !n.is_empty()) else { continue };
            let exercises: Vec<&str> = toks
                .filter_map(|t| t.strip_prefix("exercises="))
                .flat_map(|v| v.split(','))
                .filter(|s| !s.is_empty())
                .collect();
            match resolve_test(src, name) {
                // A genuine absence is already a HAZARD in obligation_gaps; warn only on
                // the looser case where a substring matched but no real definition does.
                TestRef::Absent if src.contains(name) => {
                    warns.push(format!("UNLINKED {id} — `test:{name}` matches no `fn {name}(` definition (substring only)"));
                }
                TestRef::Absent => {}
                TestRef::Ambiguous(n) => {
                    warns.push(format!("AMBIGUOUS {id} — `test:{name}` matches {n} definitions; name the one that drives the rule"));
                }
                TestRef::Found { ignored: true, .. } => {
                    warns.push(format!("IGNORED  {id} — `test:{name}` is #[ignore]'d; an ignored test never runs and discharges nothing"));
                }
                TestRef::Found { .. } if exercises.is_empty() => {
                    warns.push(format!("UNLINKED {id} — `test:{name}` declares no `exercises=` link to the rule it discharges"));
                }
                TestRef::Found { body, .. } => {
                    for tok in &exercises {
                        if !body.contains(tok) {
                            warns.push(format!("HOLLOW   {id} — `test:{name}` does not reference `{tok}` (declared via exercises=); it may not exercise the rule"));
                        }
                    }
                }
            }
        } else if disp == "structural" {
            if is_behavioural_kind(id) {
                warns.push(format!("RELABEL  {id} — `structural` on a behavioural obligation the analyse lane does not discharge; disposition it `test:` (do not relabel to dodge)"));
            }
        } else if let Some(reason) = disp.strip_prefix("waived:") {
            if reason.trim().is_empty() {
                warns.push(format!("UNWAIVED {id} — `waived:` with no reason; record why the obligation is not discharged"));
            }
        }
    }
    warns
}

/// The disposition gaps between the derived obligations and the manifest:
/// `MISSING` (obligation with no disposition), `STALE` (disposition for a
/// no-longer-derived obligation), and `ABSENT` (a `test:<name>` not in the test
/// sources, or a deep-verification `kani:`/`apalache:`/`tlaps:` proof name not in
/// the prove sources, when those are supplied). Pure, so it is unit-tested.
fn obligation_gaps(
    plan_ids: &[String],
    dispositions: &[(String, String)],
    tests: Option<&str>,
    prove: Option<&str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for id in plan_ids {
        match dispositions.iter().find(|(k, _)| k == id) {
            None => problems.push(format!("MISSING  {id} — no disposition")),
            Some((_, disp)) => {
                if let (Some(rest), Some(src)) = (disp.strip_prefix("test:"), tests) {
                    // The name is the first token; `exercises=` and other suffixes follow it.
                    let name = rest.split_whitespace().next().unwrap_or("");
                    if !name.is_empty() && !src.contains(name) {
                        problems.push(format!("ABSENT   {id} — `test:{name}` not in the test sources"));
                    }
                }
                // Deep-verification tiers (host-prove): a proof discharges the
                // obligation. When prove sources are supplied, the named harness /
                // invariant / theorem must occur in them — the analog of `test:`.
                for pfx in ["kani:", "apalache:", "tlaps:"] {
                    if let (Some(rest), Some(src)) = (disp.strip_prefix(pfx), prove) {
                        // The proof NAME is the first token; `bound=`/`spec=`/`inputs=` follow it.
                        let name = rest.split_whitespace().next().unwrap_or("");
                        if !name.is_empty() && !src.contains(name) {
                            problems.push(format!("ABSENT   {id} — `{pfx}{name}` not in the prove sources"));
                        }
                    }
                }
            }
        }
    }
    for (id, _) in dispositions {
        if !plan_ids.iter().any(|p| p == id) {
            problems.push(format!("STALE    {id} — dispositioned but no longer an obligation"));
        }
    }
    problems
}

/// A deep-verification rung disposition: `<tool>:<name> [bound=<b>] [spec=<file>]`,
/// e.g. `kani:verify_x bound=unwind=20` or `apalache:Inv spec=Scan.tla bound=length=12`.
struct Rung {
    tool: String,           // kani | apalache | tlaps
    name: String,           // harness | invariant | module
    bound: Option<String>,  // host-prove bound string, e.g. "unwind=20" / "length=12"
    spec: Option<String>,   // apalache/tlaps spec/module file (relative to the --rederive dir)
    inputs: Vec<String>,    // files the proof consumes — the offline staleness signal (call/0018)
}

/// Parse a rung disposition; `None` if it is not a `kani:`/`apalache:`/`tlaps:` one.
fn parse_rung(disp: &str) -> Option<Rung> {
    let mut toks = disp.split_whitespace();
    let (tool, name) = toks.next()?.split_once(':')?;
    if !["kani", "apalache", "tlaps"].contains(&tool) || name.is_empty() {
        return None;
    }
    // Strip a `"..."` wrapper from each disposition token; a mis-quoted token is kept raw and
    // fails downstream on a not-found spec (issue #6). parse_rung never exits — a malformed
    // disposition already returns None — so this stays tolerant.
    let unq = |s: &str| unquote_recipe_token(s).unwrap_or_else(|| s.to_string());
    let mut rung = Rung { tool: tool.into(), name: unq(name), bound: None, spec: None, inputs: Vec::new() };
    for t in toks {
        if let Some(v) = t.strip_prefix("bound=") {
            rung.bound = Some(unq(v));
        } else if let Some(v) = t.strip_prefix("spec=") {
            rung.spec = Some(unq(v));
        } else if let Some(v) = t.strip_prefix("inputs=") {
            rung.inputs = v.split(',').filter(|s| !s.is_empty()).map(&unq).collect();
        }
    }
    Some(rung)
}

/// The numeric magnitude of a bound like `unwind=20` / `length=12`; `unbounded` is the max.
fn bound_value(b: &str) -> Option<u64> {
    if b == "unbounded" {
        return Some(u64::MAX);
    }
    b.rsplit('=').next()?.trim().parse().ok()
}

/// Does a host-prove verdict line **discharge** this rung (`call/0018`)? Require the tool's
/// PASS word, and — for a bounded tool with a declared bound — that the verdict's bound is at
/// least the declared one. Returns `Err(reason)` on a non-discharge. Pure, so it is unit-tested.
fn verdict_discharges(rung: &Rung, verdict: &str) -> Result<(), String> {
    let pass_word = match rung.tool.as_str() {
        "kani" => "SUCCESSFUL",
        "apalache" => "PROVEN",
        "tlaps" => "ALL-PROVED",
        other => return Err(format!("unknown rung tool `{other}`")),
    };
    if !verdict.starts_with(pass_word) {
        return Err(format!("not a PASS ({pass_word}): {verdict}"));
    }
    let Some(declared) = &rung.bound else {
        return Ok(());
    };
    // Compare the verdict's `[bound=…]` against the declared bound. TLAPS carries
    // `[unbounded]` and never needs a numeric bound.
    match verdict.split_once("[bound=").and_then(|(_, r)| r.split(']').next()) {
        Some("unspecified") => Err(format!("declared bound {declared} but the verdict bound is unspecified")),
        Some(got) => match (bound_value(got), bound_value(declared)) {
            (Some(g), Some(d)) if g >= d => Ok(()),
            (Some(g), Some(d)) => Err(format!("verdict bound {got} ({g}) < declared {declared} ({d})")),
            _ => Err(format!("uncomparable bounds: verdict `{got}`, declared `{declared}`")),
        },
        None if rung.tool == "tlaps" => Ok(()),
        None => Err(format!("declared bound {declared} but the verdict carries none: {verdict}")),
    }
}

/// Re-run a rung's verifier via `host-prove` in `dir` and return its one verdict line.
/// host-prove must be on PATH (the verify lane installs it); the verifier is its subprocess.
fn run_host_prove(rung: &Rung, dir: &Path) -> Result<String, String> {
    let dir_s = dir.to_string_lossy().to_string();
    let mut cmd = process::Command::new("host-prove");
    match rung.tool.as_str() {
        "kani" => {
            cmd.args(["kani", "--harness", &rung.name, "--dir", &dir_s]);
        }
        "apalache" => {
            let spec = rung.spec.as_ref().ok_or_else(|| format!("apalache:{} needs `spec=<file>`", rung.name))?;
            cmd.args(["apalache", "--mode", "check", "--inv", &rung.name, "--spec"]).arg(dir.join(spec));
        }
        "tlaps" => {
            let spec = rung.spec.clone().unwrap_or_else(|| format!("{}.tla", rung.name));
            cmd.args(["tlaps", "--module"]).arg(dir.join(spec));
        }
        other => return Err(format!("unknown rung tool `{other}`")),
    }
    if let Some(b) = &rung.bound {
        cmd.args(["--bound", b]);
    }
    match cmd.output() {
        Ok(o) => {
            let line = String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string();
            if line.is_empty() {
                Err(format!("host-prove produced no verdict (on PATH? {})", String::from_utf8_lossy(&o.stderr).trim()))
            } else {
                Ok(line)
            }
        }
        Err(e) => Err(format!("could not run host-prove (install it on PATH): {e}")),
    }
}

/// `--rederive` (`call/0018`): the real discharge. Re-run every rung disposition's verifier and
/// require it to PASS at its declared bound — replacing name-presence (`#8`). `UNPROVEN` per gap.
fn rederive_problems(dispositions: &[(String, String)], dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for (id, disp) in dispositions {
        let Some(rung) = parse_rung(disp) else {
            continue;
        };
        match run_host_prove(&rung, dir).and_then(|v| verdict_discharges(&rung, &v).map(|()| v)) {
            Ok(v) => println!("proved   {id} — {}", v.trim()),
            Err(reason) => problems.push(format!("UNPROVEN {id} — {reason}")),
        }
    }
    problems
}

/// The digest ledger path for a manifest: `<manifest>.digests` (e.g.
/// `host-lint.obligations.digests`). Tool-written by `--record-digests`, committed
/// next to the manifest as the proof's input fingerprint.
fn digest_ledger_path(manifest: &Path) -> PathBuf {
    let mut s = manifest.as_os_str().to_owned();
    s.push(".digests");
    PathBuf::from(s)
}

/// The combined `git hash-object` digest of a rung's declared `inputs`, resolved
/// relative to `base`. A missing input or a git failure is itself a change signal
/// (returned as `Err`, surfaced as STALE).
fn input_digest(inputs: &[String], base: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for inp in inputs {
        let p = base.join(inp);
        let out = process::Command::new("git").arg("hash-object").arg(&p).output()
            .map_err(|e| format!("git hash-object failed: {e}"))?;
        if !out.status.success() {
            return Err(format!("cannot hash {} ({})", p.display(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        parts.push(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    Ok(parts.join(","))
}

/// Read the digest ledger into `(obligation-id, digest)` pairs; `#` comments and
/// blank lines ignored. A missing ledger yields an empty set (the feature is opt-in).
fn read_digest_ledger(path: &Path) -> Vec<(String, String)> {
    let Ok(t) = fs::read_to_string(path) else { return Vec::new() };
    t.lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.split_once('\t').map(|(a, b)| (a.trim().to_string(), b.trim().to_string())))
        .collect()
}

/// Offline staleness (`call/0018`'s cheap signal): a rung whose declared `inputs`
/// no longer hash to the digest recorded at its last re-derivation is **STALE** —
/// the proof must be re-run. A rung with no ledger entry yet is a note, not a
/// failure (it is simply not opted into tracking until `--record-digests`). With no
/// ledger at all, the check is a no-op.
fn staleness_problems(dispositions: &[(String, String)], base: &Path, ledger: &Path) -> Vec<String> {
    let recorded = read_digest_ledger(ledger);
    if recorded.is_empty() {
        return Vec::new();
    }
    let mut problems = Vec::new();
    for (id, disp) in dispositions {
        let Some(rung) = parse_rung(disp) else { continue };
        if rung.inputs.is_empty() {
            continue;
        }
        let Some((_, want)) = recorded.iter().find(|(rid, _)| rid == id) else {
            println!("note     {id} — inputs declared but not recorded (run --record-digests)");
            continue;
        };
        match input_digest(&rung.inputs, base) {
            Ok(got) if &got == want => {}
            Ok(_) => problems.push(format!("STALE    {id} — inputs changed since last re-derivation; re-derive + --record-digests")),
            Err(e) => problems.push(format!("STALE    {id} — {e}")),
        }
    }
    problems
}

/// Write the digest ledger: the `git hash-object` fingerprint of every rung's
/// declared `inputs`. Called by `--record-digests` only after a clean `--rederive`,
/// so the recorded fingerprint always corresponds to a passing proof.
fn record_digests(dispositions: &[(String, String)], base: &Path, ledger: &Path) -> Result<usize, String> {
    let mut lines = vec![
        "# host-lifecycle obligations digest ledger (--record-digests).".to_string(),
        "# <obligation-id>\\t<git hash-object of declared inputs at re-derivation>".to_string(),
    ];
    let mut n = 0;
    for (id, disp) in dispositions {
        let Some(rung) = parse_rung(disp) else { continue };
        if rung.inputs.is_empty() {
            continue;
        }
        lines.push(format!("{id}\t{}", input_digest(&rung.inputs, base)?));
        n += 1;
    }
    let mut out = lines.join("\n");
    out.push('\n');
    fs::write(ledger, out).map_err(|e| format!("cannot write {}: {e}", ledger.display()))?;
    Ok(n)
}

/// Collect spec files (`.allium`/`.tla`/`.cfg`) under `dir`, recursively.
fn collect_spec_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_spec_files(&p, out);
            } else if matches!(p.extension().and_then(|x| x.to_str()), Some("allium" | "tla" | "cfg")) {
                out.push(p);
            }
        }
    }
}

/// `#12` (specs co-locate with software, plan/0012): a spec under `plan/*/spec/` evades the
/// mandatory lanes, which run in the software repo — so it is a HAZARD. The spec belongs with
/// its software, not in the host's plan room. Returns the count of offending spec files.
fn plan_spec_problems(root: &Path) -> usize {
    let plan = root.join("plan");
    let Ok(milestones) = fs::read_dir(&plan) else {
        return 0;
    };
    let mut bad = 0;
    for m in milestones.flatten() {
        let spec_dir = m.path().join("spec");
        if !spec_dir.is_dir() {
            continue;
        }
        let mut specs = Vec::new();
        collect_spec_files(&spec_dir, &mut specs);
        for f in specs {
            let rel = f.strip_prefix(root).unwrap_or(&f);
            println!(
                "HAZARD   {} — a spec under plan/*/spec/ evades the mandatory lanes; co-locate it with its software (plan/0012, #12)",
                rel.display()
            );
            bad += 1;
        }
    }
    bad
}

/// Pull obligation ids from `allium plan` JSON without a JSON dependency: each
/// obligation carries a `"id": "<value>"` line.
fn extract_obligation_ids(json: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in json.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("\"id\":") {
            let v = rest.trim().trim_end_matches(',').trim().trim_matches('"');
            if !v.is_empty() && !ids.iter().any(|x| x == v) {
                ids.push(v.to_string());
            }
        }
    }
    ids
}

/// Parse a `.obligations` manifest: `<id> => <disposition>` per line, `#` comments
/// and blanks skipped. Returns `(id, disposition)` pairs.
fn parse_obligation_manifest(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((id, disp)) = t.split_once("=>") {
            let id = id.trim();
            if !id.is_empty() {
                out.push((id.to_string(), disp.trim().to_string()));
            }
        }
    }
    out
}

/// Concatenate every file under `dir` (recursively), for substring checks. Walks
/// symlink-safely (host-lifecycle#15); an empty skip list preserves the exact file
/// set the input digest is taken over.
fn read_dir_recursive(dir: &Path) -> String {
    let mut text = String::new();
    for p in walk_files_safe(dir, &[]) {
        if let Ok(t) = fs::read_to_string(&p) {
            text.push_str(&t);
            text.push('\n');
        }
    }
    text
}

/// One entry in the template's `UPGRADING.md`: an action that became required at a
/// given template revision, to be applied when upgrading a repo stamped older.
struct Upgrade {
    revision: String,
    title: String,
    action: String,
    requires: String,
    /// `independent = true` — applies with no prerequisite ledger entry.
    independent: bool,
    /// `depends = <id> …` — ledger entries that must be applied first (logical
    /// prerequisite, distinct from `requires` which is a tool-version floor).
    depends: Vec<String>,
    /// `verify = <command>` — a machine-checkable post-condition for the entry's
    /// action, run by `--record` (a shell command in the repo root; non-zero
    /// refuses the record). Empty when the action has none — then recording
    /// requires an explicit `--unverified call/NNNN` citation.
    verify: String,
    /// `restates = <kind> …` (plan/0036) — the reconcile-assertion kinds this entry's
    /// spine move stales in a project's own restatements (a `RECONCILE_KINDS` subset).
    /// A non-empty `restates` marks a drift-capable entry: the reconcile arm re-reads
    /// the named restatement kinds after the entry is applied. Empty for an entry that
    /// moves no mirrorable concept.
    restates: Vec<String>,
}

/// `upgrade <dir>` — list the template `UPGRADING.md` actions newer than the repo's
/// `.host` stamp. The mechanical half of a case-(c) version upgrade: it
/// answers "since the revision I adopted, what must I do?" by git ancestry, so a
/// doc diff is no longer the only signal for the structural migrations a revision
/// span introduced.
/// Resolve a `--record` argument to a full ledger id: a 1-based ledger ordinal, an
/// exact id, or an unambiguous ≥4-char prefix. Frees a low-reliability agent from
/// retyping a hex SHA exactly, and rejects unknown/ambiguous input rather than
/// recording a lie.
fn resolve_ledger_id(input: &str, ledger_ids: &[String]) -> Result<String, String> {
    if let Ok(n) = input.parse::<usize>() {
        return (n >= 1)
            .then(|| ledger_ids.get(n - 1).cloned())
            .flatten()
            .ok_or_else(|| format!("ordinal out of range (1..={})", ledger_ids.len()));
    }
    if ledger_ids.iter().any(|x| x == input) {
        return Ok(input.to_string());
    }
    if input.len() >= 4 {
        let m: Vec<&String> = ledger_ids.iter().filter(|x| x.starts_with(input)).collect();
        match m.len() {
            1 => return Ok(m[0].clone()),
            n if n > 1 => return Err(format!("ambiguous prefix (matches {n} entries)")),
            _ => {}
        }
    }
    Err("unknown ledger id".to_string())
}

/// Run an entry's `verify` post-condition (a shell command) in the repo root;
/// `true` only on a zero exit. The maintainer-authored ledger command is trusted
/// the way a CI step is.
fn run_verify(root: &Path, cmd: &str) -> bool {
    let (sh, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    process::Command::new(sh)
        .arg(flag)
        .arg(cmd)
        .current_dir(root)
        // Re-entrancy guard: a `verify` that invokes `software --check` would re-run
        // the verifies; a nested check sees this and skips its own re-check.
        .env("HOST_LIFECYCLE_IN_CHECK", "1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Write `content` to `path` atomically (temp file + rename), so a crash or full
/// disk during a stamp update can never leave a truncated/empty `.host`.
fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("stamp");
    let tmp = path.with_file_name(format!("{name}.tmp"));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)
}

fn upgrade(args: &[String]) {
    let mut dir = ".";
    let mut next_only = false;
    let mut advance = false;
    let mut record: Option<&str> = None;
    let mut unverified: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--next" => next_only = true,
            "--advance" => advance = true,
            "--record" => {
                record = args.get(i + 1).map(String::as_str);
                if record.is_none() {
                    eprintln!("host-lifecycle: --record needs an <id|ordinal>");
                    process::exit(2);
                }
                i += 1;
            }
            "--unverified" => {
                unverified = args.get(i + 1).map(String::as_str);
                if unverified.is_none() {
                    eprintln!("host-lifecycle: --unverified needs a call/NNNN citation");
                    process::exit(2);
                }
                i += 1;
            }
            s if s.starts_with("--") => {
                eprintln!("host-lifecycle: unknown upgrade flag {s}");
                process::exit(2);
            }
            s => dir = s,
        }
        i += 1;
    }
    let root = match fs::canonicalize(Path::new(dir)) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    let stamp = match fs::read_to_string(root.join(STAMP)) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("host-lifecycle: no {STAMP} — not an adopted repo");
            process::exit(2);
        }
    };
    let Some(template) = find_template_dir(&root) else {
        eprintln!("host-lifecycle: cannot find the template — `upgrade` reads UPGRADING.md from");
        eprintln!("  <root>/host-template/ or a registered submodule carrying it. Register it,");
        eprintln!("  then check it out at the revision you are upgrading to:");
        eprintln!("    git submodule add {TEMPLATE_URL} host-template");
        eprintln!("    (cd host-template && git checkout <target-revision>)");
        eprintln!("  (if you adopted from a fork, use the `template =` URL recorded in {STAMP})");
        process::exit(2);
    };
    let text = match fs::read_to_string(template.join("UPGRADING.md")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("host-lifecycle: cannot read UPGRADING.md: {e}");
            process::exit(2);
        }
    };
    let entries = parse_upgrading(&text);
    let ledger_problems = validate_ledger(&entries);
    if !ledger_problems.is_empty() {
        for p in &ledger_problems {
            eprintln!("host-lifecycle: ledger: {p}");
        }
        process::exit(1);
    }
    let ledger_ids: Vec<String> = entries.iter().map(|e| e.revision.clone()).collect();
    let applied = read_applied_ids(&root);

    // Determine the baseline; migrate a legacy single-`revision` stamp once.
    let baseline = match baseline_field(&stamp) {
        Some(b) => b,
        None => {
            let Some(rev) = parse_revision(&stamp) else {
                eprintln!("host-lifecycle: {STAMP} has neither `baseline` nor `revision`");
                process::exit(2);
            };
            let Some(b) = derive_baseline(&template, &ledger_ids, &rev) else {
                eprintln!("host-lifecycle: cannot map revision {} to a ledger baseline — fetch the template to it first", short(&rev));
                process::exit(2);
            };
            let migrated = set_stamp_field(&stamp, "baseline", &b);
            if let Err(e) = write_atomic(&root.join(STAMP), &migrated) {
                eprintln!("host-lifecycle: cannot write migrated {STAMP}: {e}");
                process::exit(2);
            }
            println!("migrated stamp: revision {} -> baseline {}", short(&rev), short(&b));
            b
        }
    };
    let base = Some(baseline.as_str());
    let is_applied = |id: &str| entry_applied(id, &ledger_ids, base, &applied);

    // Consistency: an applied entry whose declared `depends` is unapplied is a corrupt
    // record — fail loud (the teeth a membership set alone lacks).
    let mut inconsistent = false;
    for e in &entries {
        if is_applied(&e.revision) {
            for d in &e.depends {
                if !is_applied(d) {
                    eprintln!("host-lifecycle: INCONSISTENT — {} is applied but its dependency {} is not", short(&e.revision), short(d));
                    inconsistent = true;
                }
            }
        }
    }
    if inconsistent {
        process::exit(1);
    }

    // --record <id|ordinal>: record a *verified* claim that an entry was applied
    // (plan/0022 step 4). The tool validates the id, gates on dependencies, runs the
    // entry's `verify` post-condition (or demands an explicit `--unverified call/NNNN`
    // citation when it has none), and appends an append-only provenance line — so a
    // low-reliability agent never hand-edits the stamp and a bare claim cannot bury work.
    if let Some(input) = record {
        let id = match resolve_ledger_id(input, &ledger_ids) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("host-lifecycle: --record {input}: {e}");
                process::exit(2);
            }
        };
        let entry = entries.iter().find(|e| e.revision == id).expect("resolved id is a ledger entry");
        if is_applied(&id) {
            println!("already applied: {} ({})", short(&id), entry.title);
            return;
        }
        let unmet: Vec<String> = entry.depends.iter().filter(|d| !is_applied(d)).map(|d| short(d).to_string()).collect();
        if !unmet.is_empty() {
            eprintln!("host-lifecycle: refuse — {} depends on unapplied {}", short(&id), unmet.join(" "));
            process::exit(1);
        }
        let via = if !entry.verify.is_empty() {
            if !run_verify(&root, &entry.verify) {
                eprintln!("host-lifecycle: refuse — the verify post-condition for {} failed: {}", short(&id), entry.verify);
                process::exit(1);
            }
            "verify".to_string()
        } else {
            let Some(cite) = unverified else {
                eprintln!("host-lifecycle: refuse — {} declares no `verify`; recording an unverifiable claim needs `--unverified call/NNNN` (a decision authorizing it)", short(&id));
                process::exit(1);
            };
            if !cited_decision_exists(&root, cite) {
                eprintln!("host-lifecycle: refuse — cited decision {cite} not found under call/");
                process::exit(1);
            }
            cite.to_string()
        };
        // Append-only provenance, written to wherever the applied-set lives (plan/0037):
        // `.host-receipts` once migrated (or for a fresh adoption), the legacy `.host` until
        // then, so the set never fragments across the two files.
        let af = applied_file(&root);
        let cur = fs::read_to_string(root.join(af)).unwrap_or_default();
        let new = append_stamp_line(&cur, &format!("applied = {} recorded={} via={}", id, today(), via));
        if let Err(e) = write_atomic(&root.join(af), &new) {
            eprintln!("host-lifecycle: cannot write {af}: {e}");
            process::exit(2);
        }
        let remaining = entries.iter().filter(|e| e.revision != id && !is_applied(&e.revision)).count();
        println!("recorded {} ({}) via {}; {} still pending", short(&id), entry.title, via, remaining);
        return;
    }

    // --advance: move the baseline forward over a contiguous run of already-applied
    // entries and absorb their now-redundant `applied` lines (plan/0022 step 5). The
    // guard is structural: it only ever advances across entries that are applied, so
    // it can never sweep an unapplied entry into the baseline.
    if advance {
        let pos = |x: &str| ledger_ids.iter().position(|e| e == x);
        let Some(cur) = pos(&baseline) else {
            eprintln!("host-lifecycle: baseline {} is not a ledger entry", short(&baseline));
            process::exit(1);
        };
        let mut new_pos = cur;
        for (p, id) in ledger_ids.iter().enumerate().skip(cur + 1) {
            if is_applied(id) {
                new_pos = p;
            } else {
                break;
            }
        }
        if new_pos == cur {
            println!("baseline already at the furthest contiguous-applied entry ({})", short(&baseline));
            return;
        }
        let new_baseline = ledger_ids[new_pos].clone();
        let absorbed: Vec<String> = applied
            .iter()
            .filter(|a| matches!(pos(a), Some(i) if i <= new_pos))
            .cloned()
            .collect();
        // Baseline advances in the stamp; the absorbed `applied` lines are removed from
        // wherever they live (plan/0037), so this works on a legacy `.host`, a migrated
        // `.host-receipts`, or a transitional split across both.
        let cur_stamp = fs::read_to_string(root.join(STAMP)).unwrap_or_else(|_| stamp.clone());
        let s = set_stamp_field(&cur_stamp, "baseline", &new_baseline);
        let s = remove_applied_lines(&s, &absorbed);
        if let Err(e) = write_atomic(&root.join(STAMP), &s) {
            eprintln!("host-lifecycle: cannot write {STAMP}: {e}");
            process::exit(2);
        }
        if let Ok(rc) = fs::read_to_string(root.join(RECEIPTS)) {
            let rc2 = remove_applied_lines(&rc, &absorbed);
            if rc2 != rc {
                if let Err(e) = write_atomic(&root.join(RECEIPTS), &rc2) {
                    eprintln!("host-lifecycle: cannot write {RECEIPTS}: {e}");
                    process::exit(2);
                }
            }
        }
        println!("advanced baseline {} -> {}; absorbed {} applied id(s)", short(&baseline), short(&new_baseline), absorbed.len());
        return;
    }

    let pending: Vec<&Upgrade> = entries.iter().filter(|e| !is_applied(&e.revision)).collect();
    let deps_ready = |e: &Upgrade| e.depends.iter().all(|d| is_applied(d));

    if next_only {
        match pending.iter().find(|e| deps_ready(e)) {
            Some(e) => {
                println!("next: {}  {}", short(&e.revision), e.title);
                println!("  action: {}", e.action);
                if !e.requires.is_empty() {
                    println!("  requires: {}", e.requires);
                }
                println!("  record after applying: host-lifecycle upgrade --record {} {dir}", short(&e.revision));
            }
            None if pending.is_empty() => println!("up to date (baseline {}, {} applied out of order)", short(&baseline), applied.len()),
            None => println!("blocked: {} pending, none with all dependencies applied", pending.len()),
        }
        return;
    }

    if pending.is_empty() {
        println!("up to date (baseline {}, {} applied out of order)", short(&baseline), applied.len());
        return;
    }
    println!("baseline {} — {} pending:", short(&baseline), pending.len());
    for e in &pending {
        let dep = if e.independent {
            "  [independent]".to_string()
        } else if e.depends.is_empty() {
            String::new()
        } else {
            let unmet: Vec<String> = e.depends.iter().filter(|d| !is_applied(d)).map(|d| short(d).to_string()).collect();
            if unmet.is_empty() {
                "  [deps ok]".to_string()
            } else {
                format!("  [blocked on: {}]", unmet.join(" "))
            }
        };
        println!("  {}  {}{}", short(&e.revision), e.title, dep);
    }
}

/// Parse `UPGRADING.md`: `[upgrade "<revision>"]` stanzas with `title`, `action`,
/// and `requires` keys (git-config-style; `#` comments and blanks ignored).
fn parse_upgrading(text: &str) -> Vec<Upgrade> {
    let mut out: Vec<Upgrade> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(rev) = t.strip_prefix("[upgrade \"").and_then(|r| r.strip_suffix("\"]")) {
            out.push(Upgrade {
                revision: rev.to_string(),
                title: String::new(),
                action: String::new(),
                requires: String::new(),
                independent: false,
                depends: Vec::new(),
                verify: String::new(),
                restates: Vec::new(),
            });
            continue;
        }
        let Some((key, val)) = t.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());
        let Some(cur) = out.last_mut() else {
            continue;
        };
        match key {
            "title" => cur.title = val.to_string(),
            "action" => cur.action = val.to_string(),
            "requires" => cur.requires = val.to_string(),
            "independent" => cur.independent = val.eq_ignore_ascii_case("true"),
            "depends" => cur.depends = val.split_whitespace().map(String::from).collect(),
            "verify" => cur.verify = val.to_string(),
            "restates" => cur.restates = val.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
            _ => {}
        }
    }
    out
}

/// Validate the ledger's dependency declarations. Returns one message per problem:
/// a self-dependency, an entry both `independent` and `depends`, a `depends` naming
/// an entry not in the ledger, or a dependency cycle.
fn validate_ledger(entries: &[Upgrade]) -> Vec<String> {
    let ids: std::collections::HashSet<&str> = entries.iter().map(|e| e.revision.as_str()).collect();
    let mut problems = Vec::new();
    for e in entries {
        if e.independent && !e.depends.is_empty() {
            problems.push(format!("{}: both `independent` and `depends`", short(&e.revision)));
        }
        for k in &e.restates {
            if !RECONCILE_KINDS.contains(&k.as_str()) {
                problems.push(format!("{}: restates unknown kind `{k}` (known: {})", short(&e.revision), RECONCILE_KINDS.join(", ")));
            }
        }
        for d in &e.depends {
            if d == &e.revision {
                problems.push(format!("{}: depends on itself", short(&e.revision)));
            } else if !ids.contains(d.as_str()) {
                problems.push(format!("{}: depends on unknown entry {}", short(&e.revision), short(d)));
            }
        }
    }
    // cycle detection over the depends graph (DFS with a recursion stack)
    let by_id: std::collections::HashMap<&str, &Upgrade> =
        entries.iter().map(|e| (e.revision.as_str(), e)).collect();
    let mut state: std::collections::HashMap<&str, u8> = std::collections::HashMap::new(); // 0=open,1=in-stack,2=done
    fn dfs<'a>(
        id: &'a str,
        by_id: &std::collections::HashMap<&'a str, &'a Upgrade>,
        state: &mut std::collections::HashMap<&'a str, u8>,
    ) -> bool {
        match state.get(id) {
            Some(1) => return true,
            Some(2) => return false,
            _ => {}
        }
        state.insert(id, 1);
        if let Some(e) = by_id.get(id) {
            for d in &e.depends {
                if by_id.contains_key(d.as_str()) && dfs(d.as_str(), by_id, state) {
                    return true;
                }
            }
        }
        state.insert(id, 2);
        false
    }
    let mut saw_cycle = false;
    for e in entries {
        if dfs(e.revision.as_str(), &by_id, &mut state) {
            saw_cycle = true;
        }
    }
    if saw_cycle {
        problems.push("dependency cycle in the ledger".to_string());
    }
    problems
}

/// The newest ledger entry (latest in file order) whose commit is an ancestor-or-equal
/// of `revision` in the template — the baseline a legacy single-`revision` stamp maps
/// to. Uses git ONCE, at migration time only (never in the applied/pending hot path).
fn derive_baseline(template: &Path, ledger_ids: &[String], revision: &str) -> Option<String> {
    for id in ledger_ids.iter().rev() {
        let resolves = git_out(template, &["rev-parse", "--verify", &format!("{id}^{{commit}}")]).is_some();
        if resolves && git_ok(template, &["merge-base", "--is-ancestor", id, revision]) {
            return Some(id.clone());
        }
    }
    None
}

/// The submodule path that carries `UPGRADING.md` (the template).
fn find_template_dir(root: &Path) -> Option<PathBuf> {
    for p in submodule_paths(root) {
        let cand = root.join(&p);
        if cand.join("UPGRADING.md").is_file() {
            return Some(cand);
        }
    }
    let conv = root.join("host-template");
    conv.join("UPGRADING.md").is_file().then_some(conv)
}

/// What the host-template's prose CI pins host-lifecycle at (call/0038). A `cargo install …
/// host-lifecycle --rev <sha>` line yields `Rev`; a host-lifecycle install with no parseable
/// `--rev` yields `InstallNoRev` (fail closed, so a `--rev=<sha>`/`--tag` form or a dropped pin
/// does not pass silently); no host-lifecycle install yields `NoInstall` (legitimately inert).
enum TemplatePin {
    NoInstall,
    Rev(String),
    InstallNoRev,
}

/// Read the host-lifecycle pin out of the template's prose-CI text. Pure, so it is unit-tested.
fn template_hostlc_pin(prose_yaml: &str) -> TemplatePin {
    let mut saw_install = false;
    for line in prose_yaml.lines() {
        if !(line.contains("cargo install") && line.contains("host-lifecycle")) {
            continue;
        }
        saw_install = true;
        let mut toks = line.split_whitespace();
        while let Some(t) = toks.next() {
            if t == "--rev" {
                if let Some(sha) = toks.next() {
                    return TemplatePin::Rev(sha.trim_matches('"').to_string());
                }
            } else if let Some(sha) = t.strip_prefix("--rev=") {
                return TemplatePin::Rev(sha.trim_matches('"').to_string());
            }
        }
    }
    if saw_install {
        TemplatePin::InstallNoRev
    } else {
        TemplatePin::NoInstall
    }
}

/// Whether two hex commit ids designate the same commit: a case-insensitive prefix match (git
/// abbreviates SHAs), with a 7-char floor so a too-short id cannot match everything.
fn sha_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    let n = a.len().min(b.len());
    n >= 7 && a[..n] == b[..n]
}

/// A host-* tool's commit pinned in host-template's git tree at `subpath` (a submodule gitlink),
/// read from HEAD via `git -C <template> rev-parse HEAD:<subpath>`. Reads the recorded pin from the
/// tree, so it works even when the submodule is not checked out. `None` when the path is not a
/// gitlink in HEAD or git cannot resolve it.
fn template_submodule_pin(template: &Path, subpath: &str) -> Option<String> {
    git_out(template, &["rev-parse", &format!("HEAD:{subpath}")])
}

/// One template submodule gitlink checked against the recorded pin (call/0038). A gitlink that is
/// present and drifted is 1 HAZARD; an absent submodule (the template does not carry the tool) is
/// inert.
fn submodule_pin_problem(template: &Path, subpath: &str, tool: &str, want: &str) -> usize {
    match template_submodule_pin(template, subpath) {
        Some(sub) if !sha_eq(&sub, want) => {
            println!(
                "HAZARD   host-template {subpath} submodule pins {tool} {} but .host-software gates on {} (call/0038: bump the submodule on release)",
                short(&sub),
                short(want)
            );
            1
        }
        _ => 0,
    }
}

/// call/0038: the host-template must pin every host-* tool it carries at the same commit the host
/// gates on, so a tool release that does not fully upgrade the template is caught. host-template
/// pins host-lifecycle in TWO places (the prose-CI `--rev` install and the `tools/host-lifecycle`
/// submodule) and host-lint in one (`tools/host-lint`); each is checked against the recorded
/// `.host-software` pin. Returns the count of drifted pins (0 clean). Inert (0) when this repo
/// carries no template submodule, or names no such recipe pin. The whole-suite `software --check`
/// (no `--item`) is the enforcing gate; a component-narrowed check legitimately skips this global
/// invariant.
/// The submodule names under `tools/` in the template's `.gitmodules` — the distribution
/// surface the template ships to adopters (host-* tools plus the external referenced tools
/// allium/specula). `find_template_dir` already gated on materialization, so a direct read.
fn tools_submodule_names(template: &Path) -> Vec<String> {
    let text = match fs::read_to_string(template.join(".gitmodules")) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("path")
                .and_then(|r| r.trim_start().strip_prefix('='))
                .map(|v| v.trim())
                .and_then(|p| p.strip_prefix("tools/").map(|n| n.to_string()))
        })
        .collect()
}

/// The carried template tools (call/0038): each `.host-software` component that also
/// appears as a `host-template/tools/<name>` submodule — a tool developed here that the
/// template also distributes. External referenced tools (allium, specula) are in `tools/`
/// but not `.host-software`, so they are excluded; a component not in `tools/` is not
/// distributed. Inert (empty) for a consumer that develops no host-* tool or carries no
/// template. plan/0069: this derived intersection replaces the prior hardcoded list.
fn carried_template_tools(root: &Path, recipe: &[Software]) -> Vec<String> {
    let Some(tdir) = find_template_dir(root) else {
        return Vec::new();
    };
    let names = tools_submodule_names(&tdir);
    recipe
        .iter()
        .map(|s| s.name.clone())
        .filter(|n| names.iter().any(|x| x == n))
        .collect()
}

/// call/0038: the host-template must pin every host-* tool it carries at the same commit
/// the host gates on, so a tool release that does not fully upgrade the template is caught.
/// The carried set is derived (`carried_template_tools`: `.host-software` ∩ `tools/`), so a
/// new family tool lands in both lanes with no code change. host-lifecycle carries a second
/// surface — the prose-CI `--rev` install (a mechanical pin, not prose) — checked as a
/// labelled special case; every carried tool's `tools/<name>` submodule gitlink is checked
/// in the loop. Returns the drift count (0 clean). Inert (0) when this repo carries no
/// template submodule. The whole-suite `software --check` (no `--item`) is the enforcing
/// gate; a component-narrowed check legitimately skips this global invariant.
fn template_pin_problems(root: &Path, recipe: &[Software]) -> usize {
    let Some(tdir) = find_template_dir(root) else {
        return 0;
    };
    let pin_of = |name: &str| recipe.iter().find(|s| s.name == name).map(|s| s.pin.as_str());
    let mut bad = 0usize;

    // host-lifecycle is also installed via the prose-CI `--rev` (the prose-gate tool — a
    // methodology fact, the one workflow-pin surface). A labelled special case; the loop
    // below checks host-lifecycle's tools/host-lifecycle submodule like any carried tool.
    if let Some(hl) = pin_of("host-lifecycle") {
        if let Ok(yaml) = fs::read_to_string(tdir.join(".github").join("workflows").join("prose.yml")) {
            match template_hostlc_pin(&yaml) {
                TemplatePin::NoInstall => {}
                TemplatePin::Rev(rev) if sha_eq(&rev, hl) => {}
                TemplatePin::Rev(rev) => {
                    println!(
                        "HAZARD   host-template prose.yml pins host-lifecycle {} but .host-software gates on {} (call/0038: bump the template pin on release)",
                        short(&rev),
                        short(hl)
                    );
                    bad += 1;
                }
                TemplatePin::InstallNoRev => {
                    println!(
                        "HAZARD   host-template prose.yml installs host-lifecycle but pins no --rev commit (call/0038: pin it to {})",
                        short(hl)
                    );
                    bad += 1;
                }
            }
        }
    }

    // Every carried tool: its tools/<name> submodule gitlink must match the recorded pin.
    for tool in carried_template_tools(root, recipe) {
        if let Some(want) = pin_of(&tool) {
            bad += submodule_pin_problem(&tdir, &format!("tools/{tool}"), &tool, want);
        }
    }

    bad
}

/// Root-level `.md` files the book places in a specific room (so the catch-all
/// Reference section does not list them twice).
const PLACED_ROOT_MD: [&str; 7] = ["SUMMARY.md", "README.md", "MEMORY.md", "CLAUDE.md", "PLAN.md", "home.md", "index.md"];

/// A published section of the book — one per room, emitted in lifecycle order
/// (Who → What/When → Where → Why → How → Memory). A section with no content page
/// fails `book --check` (the stub-coverage gate).
struct Section {
    /// The SUMMARY part-title, e.g. "Cast: who".
    title: String,
    /// The room this covers, named in a coverage failure.
    room: &'static str,
    /// The room has source material, so it MUST render a page — `--check` fails if
    /// it does not (the generator dropped a room, or rendered a content-free page).
    /// A room with no source (a fresh `call/`, a project with no `.host-software`)
    /// is legitimately empty and not gated.
    required: bool,
    pages: Vec<Page>,
}

/// One rendered page: where it lands under `mdBook/src/`, its sidebar label and indent,
/// and how to produce it.
struct Page {
    /// Path under `mdBook/src/`, e.g. `cast/mara.md`.
    dest: String,
    /// Sidebar label.
    label: String,
    /// SUMMARY indent depth: 0 top-level, 1 nested, …
    depth: usize,
    body: PageBody,
}

/// How a page's body is produced: copy a source file verbatim, or write generated
/// markdown (the Where stub, a spec page, a spec index).
enum PageBody {
    Copy(PathBuf),
    Inline(String),
}

/// `book <dir> [--dry-run]` — generate `book.toml` + `mdBook/src/` (SUMMARY in lifecycle
/// order, specs rendered, a Where stub from `.host-software`). `book --check <dir>`
/// fails unless every room renders at least one page with content. The methodology
/// mandates five rooms and two spec formats but shipped no canonical way to publish
/// them; this is that one maintained publisher, so adopters do not hand-roll a
/// generator that drops rooms or re-derives the `call/0005` src-scoping wrong.
fn book(args: &[String]) {
    let mut check = false;
    let mut dry = false;
    let mut print_mount = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "--dry-run" => dry = true,
            "--print-mount" => print_mount = true,
            _ => pos.push(a),
        }
    }
    let Some(dir) = pos.first() else {
        eprintln!("host-lifecycle book <dir> [--check] [--dry-run] [--print-mount]");
        process::exit(2);
    };
    let root = match fs::canonicalize(Path::new(dir.as_str())) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    // host#17: the reference Site workflow reads the declared mount from the tool
    // (the internalise-tool-orchestration doctrine), rather than shell-grepping `.host`.
    if print_mount {
        println!("{}", stamp_book_mount(&root));
        return;
    }
    let sections = plan_book(&root);
    if check {
        book_check(&sections);
    } else {
        let home = home_page(&root, &stamp_title(&root), &sections);
        write_book(&root, &home, &sections, dry);
    }
}

/// Build the six sections in lifecycle order. Pure: reads the repo, writes nothing,
/// so `--check` and generation see the same plan.
fn plan_book(root: &Path) -> Vec<Section> {
    segregate_records(vec![
        flat_room(root, "cast", "Cast: who", "cast"),
        plan_plan(root),
        plan_software(root),
        flat_room(root, "call", "Call: why", "call"),
        plan_reference(root),
        plan_memory(root),
    ])
}

/// A retired decision (`Status:` superseded/deprecated/rejected) gets a banner and a
/// nav-label suffix; live pages return None. `Status:` is the methodology's intentional
/// retire signal — `book` keys off it alone, not the naming-audit's `.host-lintignore`
/// (which carries unrelated meanings). Only file-backed (Copy) pages are checked.
fn record_mark(page: &Page) -> Option<(&'static str, &'static str)> {
    let src = match &page.body {
        PageBody::Copy(p) => p,
        PageBody::Inline(_) => return None,
    };
    let text = fs::read_to_string(src).unwrap_or_default();
    let status = decision_field(&text, "Status").unwrap_or_default().to_ascii_lowercase();
    if status.starts_with("superseded") {
        return Some(("> **Superseded.** Retained as immutable history, not current guidance.", " (superseded)"));
    }
    if status.starts_with("deprecated") {
        return Some(("> **Deprecated.** No longer in force; retained as history.", " (deprecated)"));
    }
    if status.starts_with("rejected") {
        return Some(("> **Rejected.** Not adopted; retained as history.", " (rejected)"));
    }
    None
}

/// Move retired decisions out of their live rooms into a trailing "Archive / Record"
/// section, each banner-marked and label-suffixed, so the book does not ship retired
/// content as current chapters (issue #8). A room emptied of live content is no longer
/// gated by `--check`.
fn segregate_records(sections: Vec<Section>) -> Vec<Section> {
    let mut live = Vec::new();
    let mut records: Vec<Page> = Vec::new();
    for mut sec in sections {
        let mut kept = Vec::new();
        for p in std::mem::take(&mut sec.pages) {
            match record_mark(&p) {
                Some((banner, suffix)) => {
                    let body = match &p.body {
                        PageBody::Copy(s) => fs::read_to_string(s).unwrap_or_default(),
                        PageBody::Inline(t) => t.clone(),
                    };
                    records.push(Page {
                        dest: p.dest,
                        label: format!("{}{}", p.label, suffix),
                        depth: 0,
                        body: PageBody::Inline(format!("{banner}\n\n{body}")),
                    });
                }
                None => kept.push(p),
            }
        }
        sec.required = sec.required && kept.iter().any(page_has_content);
        sec.pages = kept;
        live.push(sec);
    }
    if !records.is_empty() {
        for (i, p) in records.iter_mut().enumerate() {
            p.depth = usize::from(i > 0);
        }
        live.push(Section {
            title: "Archive / Record".to_string(),
            room: "archive",
            required: false,
            pages: records,
        });
    }
    live
}

/// A room that is a flat directory of `.md` files (cast, call): the first file is
/// the landing page (depth 0), the rest nest under it. README floats to the front.
fn flat_room(root: &Path, dir_name: &str, title: &str, room: &'static str) -> Section {
    let files = list_md(&root.join(dir_name));
    let mut pages = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let fname = file_name_str(f);
        let stem = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        pages.push(Page {
            dest: format!("{dir_name}/{fname}"),
            label: label_for(f, &stem),
            depth: usize::from(i > 0),
            body: PageBody::Copy(f.clone()),
        });
    }
    Section { title: title.to_string(), room, required: !pages.is_empty(), pages }
}

/// The Plan room (What/When): a landing page (root `PLAN.md` if present, else a
/// generated index), then each milestone, then its specs rendered as code pages.
fn plan_plan(root: &Path) -> Section {
    let mut pages = Vec::new();
    let plan_md = root.join("PLAN.md");
    if plan_md.is_file() {
        pages.push(Page {
            dest: "PLAN.md".to_string(),
            label: label_for(&plan_md, "Plan"),
            depth: 0,
            body: PageBody::Copy(plan_md),
        });
    } else {
        pages.push(Page {
            dest: "plan-index.md".to_string(),
            label: "Plan".to_string(),
            depth: 0,
            body: PageBody::Inline("# Plan: what & when\n\nMilestones in this project.\n".to_string()),
        });
    }
    for m in milestone_dirs(&root.join("plan")) {
        let dname = file_name_str(&m);
        let readme = m.join("README.md");
        let label = if readme.is_file() { label_for(&readme, &dname) } else { humanize(&dname) };
        let dest = format!("plan/{dname}/README.md");
        let body = if readme.is_file() {
            PageBody::Copy(readme)
        } else {
            PageBody::Inline(format!("# {}\n", humanize(&dname)))
        };
        pages.push(Page { dest, label, depth: 1, body });

        let spec_dir = m.join("spec");
        let specs = spec_files(&spec_dir);
        if !specs.is_empty() {
            // Path relative to spec/, forward-slashed, preserving <topic>/ nesting.
            let rel = |sp: &Path| {
                sp.strip_prefix(&spec_dir)
                    .unwrap_or(sp)
                    .to_string_lossy()
                    .replace('\\', "/")
            };
            let mut idx = String::from("# Specs\n\n");
            for sp in &specs {
                let r = rel(sp);
                idx.push_str(&format!("- [{r}](spec/{r}.md)\n"));
            }
            pages.push(Page {
                dest: format!("plan/{dname}/spec-index.md"),
                label: "specs".to_string(),
                depth: 2,
                body: PageBody::Inline(idx),
            });
            for sp in &specs {
                let r = rel(sp);
                let sname = file_name_str(sp);
                let ext = sp.extension().and_then(|e| e.to_str()).unwrap_or("");
                let src = fs::read_to_string(sp).unwrap_or_default();
                pages.push(Page {
                    dest: format!("plan/{dname}/spec/{r}.md"),
                    label: sname.clone(),
                    depth: 3,
                    body: PageBody::Inline(spec_page(&sname, &src, ext)),
                });
            }
        }
    }
    Section { title: "Plan: what & when".to_string(), room: "plan", required: true, pages }
}

/// The Where room: a stub generated from `.host-software` — component name, url,
/// pin, worktrees, and the materialize command. Reads only the committed recipe, so
/// it is safe in an un-materialized checkout (the worktrees themselves are never
/// walked — `call/0005`). Absent recipe → no page → `--check` reports the gap.
fn plan_software(root: &Path) -> Section {
    let mut pages = Vec::new();
    if let Ok(text) = fs::read_to_string(root.join(SOFTWARE)) {
        let recipe = parse_software(&text);
        if !recipe.is_empty() {
            pages.push(Page {
                dest: "where.md".to_string(),
                label: "Software".to_string(),
                depth: 0,
                body: PageBody::Inline(where_stub(&recipe)),
            });
        }
    }
    Section { title: "Software: where".to_string(), room: "software", required: !pages.is_empty(), pages }
}

/// The How room: `CLAUDE.md` (the operating manual), then a `reference/` dir if
/// present, then any loose root `.md` not already placed in another room — so no
/// existing top-level doc is silently dropped from the published record.
fn plan_reference(root: &Path) -> Section {
    let mut pages = Vec::new();
    let claude = root.join("CLAUDE.md");
    if claude.is_file() {
        pages.push(Page {
            dest: "CLAUDE.md".to_string(),
            label: label_for(&claude, "CLAUDE"),
            depth: 0,
            body: PageBody::Copy(claude),
        });
    }
    for f in list_md(&root.join("reference")) {
        let fname = file_name_str(&f);
        let stem = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        pages.push(Page {
            dest: format!("reference/{fname}"),
            label: label_for(&f, &stem),
            depth: 1,
            body: PageBody::Copy(f),
        });
    }
    for f in loose_root_md(root) {
        let fname = file_name_str(&f);
        let stem = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        pages.push(Page {
            dest: fname,
            label: label_for(&f, &stem),
            depth: 1,
            body: PageBody::Copy(f),
        });
    }
    Section { title: "Reference: how".to_string(), room: "reference", required: !pages.is_empty(), pages }
}

/// The Memory room: the append-only `MEMORY.md` scratchpad.
fn plan_memory(root: &Path) -> Section {
    let mut pages = Vec::new();
    let mem = root.join("MEMORY.md");
    if mem.is_file() {
        pages.push(Page {
            dest: "MEMORY.md".to_string(),
            label: "Memory".to_string(),
            depth: 0,
            body: PageBody::Copy(mem),
        });
    }
    Section { title: "Memory".to_string(), room: "memory", required: !pages.is_empty(), pages }
}

/// `--check`: every room must render at least one page with content. Exit 1 naming
/// each empty room; the gate a hand-rolled generator never had (issue #6, S5).
fn book_check(sections: &[Section]) {
    let mut missing = 0usize;
    for s in sections {
        if s.pages.iter().any(page_has_content) {
            println!("ok       {} ({} page(s))", s.room, s.pages.len());
        } else if s.required {
            println!("MISSING  {} has source but renders no page with content", s.room);
            missing += 1;
        } else {
            println!("skip     {} (no source — not gated)", s.room);
        }
    }
    if missing > 0 {
        eprintln!("-- {missing} room(s) unrendered");
        process::exit(1);
    }
    println!("-- every room with source renders at least one page");
}

/// Does a page carry real content? Inline bodies are checked directly; a copied
/// source must exist and be non-blank.
fn page_has_content(p: &Page) -> bool {
    match &p.body {
        PageBody::Inline(t) => !t.trim().is_empty(),
        PageBody::Copy(src) => fs::read_to_string(src).map(|t| !t.trim().is_empty()).unwrap_or(false),
    }
}

/// Generate `book.toml` and the mdBook source under `mdBook/src/` from the plan. The source tree
/// is rebuilt from scratch (it is generated output, gitignored), so a removed source never
/// lingers; the built HTML lands in `mdBook/out/` via `book.toml`'s `build-dir`. Both live under
/// one gitignored `mdBook/` folder, leaving `docs/` free for authored content (host-lifecycle#3).
fn write_book(root: &Path, home: &Page, sections: &[Section], dry: bool) {
    let src_dir = root.join("mdBook/src");
    let all = std::iter::once(home).chain(sections.iter().flat_map(|s| s.pages.iter()));
    if dry {
        println!("write  book.toml (dry-run)");
        println!("write  mdBook/src/SUMMARY.md (dry-run)");
        for p in all {
            println!("write  mdBook/src/{} (dry-run)", p.dest);
        }
        return;
    }
    let _ = fs::remove_dir_all(&src_dir);
    if let Err(e) = fs::create_dir_all(&src_dir) {
        eprintln!("host-lifecycle: cannot create {}: {e}", src_dir.display());
        process::exit(2);
    }
    if let Err(e) = fs::write(root.join("book.toml"), book_toml(root)) {
        eprintln!("host-lifecycle: cannot write book.toml: {e}");
        process::exit(2);
    }
    let mut count = 0usize;
    for p in all {
        let dest = src_dir.join(&p.dest);
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let res = match &p.body {
            PageBody::Copy(src) => fs::copy(src, &dest).map(|_| ()),
            PageBody::Inline(text) => fs::write(&dest, text),
        };
        if let Err(e) = res {
            eprintln!("host-lifecycle: cannot write {}: {e}", dest.display());
            process::exit(2);
        }
        count += 1;
    }
    if let Err(e) = fs::write(src_dir.join("SUMMARY.md"), summary_text(home, sections)) {
        eprintln!("host-lifecycle: cannot write mdBook/src/SUMMARY.md: {e}");
        process::exit(2);
    }
    println!("-- wrote book.toml + {count} page(s) + mdBook/src/SUMMARY.md");
}

/// The mdBook config: `src = "mdBook/src"` and `build-dir = "mdBook/out"` (the generated source
/// and HTML consolidated under one gitignored `mdBook/` folder, host-lifecycle#3; never `"."`,
/// which would walk the un-materialized worktrees — `call/0005`), the house light/navy theme, and
/// `custom.css` only if the repo ships one. `book.toml` stays at the root, so `mdbook build` still
/// runs from the root. The title is the stamp's `name` (so it is deterministic regardless of the
/// checkout directory), falling back to the directory name when the stamp carries none.
fn book_toml(root: &Path) -> String {
    let title = stamp_title(root);
    let mut s = format!(
        "[book]\nlanguage = \"en\"\nsrc = \"mdBook/src\"\ntitle = \"{title}\"\n\n[build]\nbuild-dir = \"mdBook/out\"\n\n[output.html]\ndefault-theme = \"light\"\npreferred-dark-theme = \"navy\"\n"
    );
    // host#17: a project serving a product site at the Pages root declares a `book-mount`
    // sub-path in `.host`; the generated book then carries mdBook's `site-url` so its
    // absolute links (the 404 and print pages) resolve under that sub-path. The line is
    // emitted only for a non-default mount, so book.toml stays byte-identical for every
    // project that publishes at the root.
    let mount = stamp_book_mount(root);
    if mount != "/" {
        s.push_str(&format!("site-url = \"{mount}\"\n"));
    }
    if root.join("custom.css").is_file() {
        s.push_str("additional-css = [\"custom.css\"]\n");
    }
    s
}

/// The optional `book-mount` from the `.host` stamp: the sub-path the published book is
/// served under (host#17), defaulting to "/". Read via the existing stamp reader, so it
/// needs no new parser and survives a baseline re-stamp.
fn stamp_book_mount(root: &Path) -> String {
    fs::read_to_string(root.join(STAMP))
        .ok()
        .and_then(|t| stamp_field(&t, "book-mount"))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "/".to_string())
}

/// The project name for the book title: the `.host` stamp's `name`, falling back to
/// the checkout directory name. Used for both `book.toml` and the home page.
fn stamp_title(root: &Path) -> String {
    fs::read_to_string(root.join(STAMP))
        .ok()
        .and_then(|t| stamp_field(&t, "name"))
        .or_else(|| root.file_name().and_then(|n| n.to_str()).map(String::from))
        .unwrap_or_else(|| "docs".to_string())
}

/// The home/index page — mdBook's landing, listed as a prefix chapter before the
/// first room, so no room becomes the site's front page. A repo `README.md` or
/// `home.md` (if present and non-blank) is used verbatim; otherwise a generated
/// overview links each room's landing.
fn home_page(root: &Path, name: &str, sections: &[Section]) -> Page {
    // Labelled with the project name (not a generic "Home"): the landing reads as
    // the project itself — implicit, not a dated "Home" link. mdBook renders the tab
    // title as `{label} - {book title}` and the book title is also the project name,
    // so the tab repeats it ("agentic-host - agentic-host"); that is the accepted
    // trade-off for an implicit, project-named landing.
    for cand in ["README.md", "home.md"] {
        let p = root.join(cand);
        if fs::read_to_string(&p).map(|t| !t.trim().is_empty()).unwrap_or(false) {
            return Page { dest: "index.md".to_string(), label: name.to_string(), depth: 0, body: PageBody::Copy(p) };
        }
    }
    let mut s = format!("# {name}\n\nProject documentation, organized by the methodology's rooms.\n\n");
    for sec in sections {
        if let Some(p) = sec.pages.first() {
            s.push_str(&format!("- [{}]({})\n", sec.title, served_link(&p.dest)));
        }
    }
    Page { dest: "index.md".to_string(), label: name.to_string(), depth: 0, body: PageBody::Inline(s) }
}

/// The in-content link to a generated page. mdBook serves a `README.md` page at
/// `index.html` (its README-to-index rule) but rewrites an in-content `README.md`
/// link to `README.html`, which is never generated (a 404). Link the served
/// `index.md` instead, so the generated home overview resolves (host#15).
fn served_link(dest: &str) -> String {
    match dest.strip_suffix("README.md") {
        Some(prefix) => format!("{prefix}index.md"),
        None => dest.to_string(),
    }
}

/// Render `mdBook/src/SUMMARY.md`: the home page as a prefix chapter (mdBook's landing),
/// then a `# <part>` header per section with its pages as indented list items in
/// lifecycle order.
fn summary_text(home: &Page, sections: &[Section]) -> String {
    let mut s = String::from("# Summary\n\n");
    s.push_str(&format!("[{}]({})\n\n", home.label, home.dest));
    for sec in sections {
        s.push_str(&format!("# {}\n\n", sec.title));
        for p in &sec.pages {
            let indent = "  ".repeat(p.depth);
            s.push_str(&format!("{indent}- [{}]({})\n", p.label, p.dest));
        }
        s.push('\n');
    }
    s
}

/// The Where stub markdown for a parsed `.host-software` recipe.
fn where_stub(recipe: &[Software]) -> String {
    let mut s = String::from(
        "# Software: where\n\nThe action this project produces. Each component is a bare object store \
with worktrees, not committed into this repo; the recipe below is the reproducibility \
anchor. Materialize the worktrees locally with:\n\n```\nhost-lifecycle software --materialize .\n```\n\n",
    );
    for c in recipe {
        s.push_str(&format!("## {}\n\n- url: {}\n- pin: `{}`\n", c.name, c.url, c.pin));
        let mut wts: Vec<String> = c.worktrees.clone();
        for w in &c.lines {
            wts.push(format!("{} @ {}", w.branch, short(&w.pin)));
        }
        if wts.is_empty() {
            s.push_str("- worktrees: none (single canonical line)\n");
        } else {
            s.push_str(&format!("- worktrees: {}\n", wts.join(", ")));
        }
        if !c.builds.is_empty() {
            let plats: Vec<&str> = c.builds.iter().map(|b| b.platform.as_str()).collect();
            s.push_str(&format!("- builds: {}\n", plats.join(", ")));
        }
        s.push('\n');
    }
    s
}

/// Render a spec file as a fenced code page (mdBook shows `.allium`/`.tla` as
/// preformatted text). The fence grows past any backtick run in the body so a spec
/// that itself contains a fence still renders.
fn spec_page(name: &str, body: &str, ext: &str) -> String {
    let mut longest = 0usize;
    let mut cur = 0usize;
    for ch in body.chars() {
        if ch == '`' {
            cur += 1;
            longest = longest.max(cur);
        } else {
            cur = 0;
        }
    }
    let fence = "`".repeat(longest.max(2) + 1);
    let body = body.trim_end_matches('\n');
    format!("# {name}\n\n{fence}{ext}\n{body}\n{fence}\n")
}

/// `.md` files in a directory, sorted, with `README.md` floated to the front.
fn list_md(dir: &Path) -> Vec<PathBuf> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    v.sort();
    v.sort_by_key(|p| p.file_name().and_then(|n| n.to_str()) != Some("README.md"));
    v
}

/// Numbered milestone directories under `plan/`, sorted.
fn milestone_dirs(plan: &Path) -> Vec<PathBuf> {
    let rd = match fs::read_dir(plan) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(|c: char| c.is_ascii_digit()))
                    .unwrap_or(false)
        })
        .collect();
    v.sort();
    v
}

/// Spec files (`.allium`/`.tla`/`.cfg`) under a milestone's `spec/` dir, recursively
/// (nested `spec/<topic>/` included), sorted by path (issue #7).
fn spec_files(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    collect_specs(dir, &mut v);
    v.sort();
    v
}

fn collect_specs(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for p in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
        if p.is_dir() {
            collect_specs(&p, out);
        } else if p.is_file() && is_spec_ext(p.extension().and_then(|e| e.to_str()).unwrap_or("")) {
            out.push(p);
        }
    }
}

/// Root-level `.md` files not already placed in a specific room.
fn loose_root_md(root: &Path) -> Vec<PathBuf> {
    let rd = match fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("md"))
        .filter(|p| !PLACED_ROOT_MD.contains(&p.file_name().and_then(|n| n.to_str()).unwrap_or("")))
        .collect();
    v.sort();
    v
}

/// A path's file name as an owned string (empty if it has none).
fn file_name_str(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// A page label: the file's first `# ` heading, else the humanized fallback.
fn label_for(path: &Path, fallback: &str) -> String {
    fs::read_to_string(path)
        .ok()
        .as_deref()
        .and_then(first_heading)
        .unwrap_or_else(|| humanize(fallback))
}

/// The first `# ` heading text in a markdown document.
fn first_heading(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("# ") {
            let h = rest.trim();
            if !h.is_empty() {
                return Some(h.to_string());
            }
        }
    }
    None
}

/// Turn a slug into a readable label: separators become spaces.
fn humanize(s: &str) -> String {
    s.replace(['-', '_'], " ")
}

// ---- Lifecycle manifest (plan/0025) ----------------------------------------
//
// The manifest is the single tool-readable journal of the lifecycle: one
// `[phase "<name>"]` stanza per phase, in the template root, replacing the three
// prose copies (CLAUDE.md / STRUCTURE.md / UPGRADING.md). `host-lifecycle` reads it
// for phase order, `--next`, the `book` lifecycle ordering, and the receipt gate;
// an agent reads it to see the whole lifecycle at a glance.

/// One lifecycle phase. `modality` is first-class so the spine's rule becomes
/// "every phase emits a receipt," not "every phase runs" (plan/0025 Decision A).
struct Phase {
    name: String,
    order: usize,
    /// Comma-separated modality tokens — e.g. `unconditional`,
    /// `conditional-on-Where`, `recurring-per-component`.
    modality: Vec<String>,
    command: Option<String>,
    skill: Option<String>,
    evidence: Option<String>,
    /// Phase names that must be `done` before this one (e.g. `release` requires
    /// `verify`); a dependency must sit at a lower `order`.
    requires: Vec<String>,
    /// R2 protected core: `skippable = false` refuses a `skip`/`n-a` receipt outright
    /// (`verify` and anything a green gate depends on). Defaults true.
    skippable: bool,
    /// R1 closed re-check: a command `software --check` re-executes to re-verify a
    /// `done` receipt (the analog of UPGRADING's `verify =`). A `done` whose recheck
    /// fails re-opens as a HAZARD — evidence is never self-asserted.
    recheck: Option<String>,
}

impl Phase {
    /// `recurring-per-component`: the phase runs (and is receipted) once per software
    /// component (embed/release), not once for the project.
    fn recurring(&self) -> bool {
        self.modality.iter().any(|m| m == "recurring-per-component")
    }

    /// `conditional-on-<X>` → `Some("<X>")`: the phase applies only when the project
    /// has X (e.g. `conditional-on-Where` = only with a software room). A phase that
    /// does not apply is `n-a`, tool-computed from project state, never agent-asserted.
    fn conditional_on(&self) -> Option<&str> {
        self.modality.iter().find_map(|m| m.strip_prefix("conditional-on-"))
    }
}

/// Parse a `lifecycle.manifest` (git-config style, mirrors `parse_software`): one
/// `[phase "<name>"]` stanza, then `key = val` lines. A bad `order` records 0 and is
/// surfaced by `manifest --check`; unknown keys are ignored (forward-compatible).
fn parse_manifest(text: &str) -> Vec<Phase> {
    let mut out: Vec<Phase> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(name) = t.strip_prefix("[phase \"").and_then(|r| r.strip_suffix("\"]")) {
            out.push(Phase {
                name: name.to_string(),
                order: 0,
                modality: Vec::new(),
                command: None,
                skill: None,
                evidence: None,
                requires: Vec::new(),
                skippable: true,
                recheck: None,
            });
            continue;
        }
        let Some((key, val)) = t.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());
        let Some(cur) = out.last_mut() else {
            eprintln!("host-lifecycle: {MANIFEST}:{}: `{key}` before any [phase \"...\"] stanza", i + 1);
            process::exit(2);
        };
        let list = |v: &str| v.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        match key {
            "order" => cur.order = val.parse().unwrap_or(0),
            "modality" => cur.modality = val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "command" => cur.command = Some(val.to_string()),
            "skill" => cur.skill = Some(val.to_string()),
            "evidence" => cur.evidence = Some(val.to_string()),
            "requires" => cur.requires = list(val),
            "skippable" => cur.skippable = val != "false",
            "recheck" => cur.recheck = Some(val.to_string()),
            _ => {}
        }
    }
    out
}

fn manifest(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("--check") => manifest_check(args.get(1).map(Path::new)),
        Some(p) => manifest_show(Path::new(p)),
        None => {
            eprintln!("usage: host-lifecycle manifest [--check] <path>");
            process::exit(2);
        }
    }
}

fn read_manifest_or_exit(path: &Path) -> String {
    match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("host-lifecycle: cannot read {}: {e}", path.display());
            process::exit(2);
        }
    }
}

/// `manifest <path>`: the whole lifecycle at a glance, in `order` (plan/0025 — "an
/// agent reads it to see the whole lifecycle"). Surfaces each phase's modality
/// (conditional / recurring / protected), its command, what it runs after, and the
/// evidence a `done` receipt must carry.
fn manifest_show(path: &Path) {
    let mut phases = parse_manifest(&read_manifest_or_exit(path));
    phases.sort_by_key(|p| p.order);
    for p in &phases {
        let mut tags: Vec<String> = Vec::new();
        if let Some(x) = p.conditional_on() {
            tags.push(format!("conditional-on-{x}"));
        }
        if p.recurring() {
            tags.push("recurring-per-component".into());
        }
        if !p.skippable {
            tags.push("protected".into());
        }
        let tagstr = if tags.is_empty() { String::new() } else { format!("  [{}]", tags.join(", ")) };
        println!("{:>2}. {}{tagstr}", p.order, p.name);
        if let Some(c) = &p.command {
            println!("      run: {c}");
        }
        if !p.requires.is_empty() {
            println!("      after: {}", p.requires.join(", "));
        }
        if let Some(e) = &p.evidence {
            println!("      evidence: {e}");
        }
    }
}

/// Lines that open a stanza other than `[phase "..."]` in a manifest — the
/// project-specific facts that plan/0039 moved out to `.host-software`. Returns
/// `(line-number, trimmed-stanza)` for each. `manifest --check` rejects them so the
/// spine manifest stays phases-only: separation of concerns, an overfit cannot creep
/// back into the shared spine.
fn manifest_foreign_stanzas(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let t = line.trim();
            (t.starts_with('[') && !t.starts_with("[phase \"")).then(|| (i + 1, t.to_string()))
        })
        .collect()
}

/// Structural validation of a manifest file: it is phases only (no project-specific
/// stanza); each phase carries `order`, `command` and `skill`; `order`s are unique;
/// every `requires` names a real phase that sits earlier (no forward or self
/// dependency). One `ok`/`HAZARD` line per phase; exits non-zero on any HAZARD (so a
/// CI lane can gate the template's own manifest).
fn manifest_check(path: Option<&Path>) {
    let Some(path) = path else {
        eprintln!("usage: host-lifecycle manifest --check <path>");
        process::exit(2);
    };
    let text = read_manifest_or_exit(path);
    let phases = parse_manifest(&text);
    if phases.is_empty() {
        eprintln!("HAZARD   {} has no [phase \"...\"] stanzas", path.display());
        process::exit(1);
    }
    let names: Vec<&str> = phases.iter().map(|p| p.name.as_str()).collect();
    let order_of = |n: &str| phases.iter().find(|p| p.name == n).map(|p| p.order);
    let mut hazards = 0;
    // Separation of concerns (plan/0039): the spine manifest is universal — phases only.
    // A project's own facts (its components and verifiers) live in `.host-software`, never
    // the shared spine, so no adopter inherits another project's facts and no overfit
    // creeps back in. Any stanza that is not a `[phase "..."]` is rejected.
    for (line_no, stanza) in manifest_foreign_stanzas(&text) {
        println!("HAZARD   {}:{line_no}: `{stanza}` is not a [phase] stanza — a project's facts (components, verifiers) belong in .host-software, not the spine manifest", path.display());
        hazards += 1;
    }
    for p in &phases {
        let mut problems: Vec<String> = Vec::new();
        if p.order == 0 {
            problems.push("missing or zero `order`".into());
        }
        if p.command.is_none() {
            problems.push("missing `command`".into());
        }
        if p.skill.is_none() {
            problems.push("missing `skill`".into());
        }
        if p.order != 0 && phases.iter().filter(|q| q.order == p.order).count() > 1 {
            problems.push(format!("duplicate order {}", p.order));
        }
        for r in &p.requires {
            if r == &p.name {
                problems.push("`requires` names itself".into());
            } else if !names.contains(&r.as_str()) {
                problems.push(format!("`requires` names unknown phase `{r}`"));
            } else if matches!(order_of(r), Some(ro) if ro >= p.order) {
                problems.push(format!("`requires {r}` is not earlier (order {} >= {})", order_of(r).unwrap_or(0), p.order));
            }
        }
        if problems.is_empty() {
            println!("ok       phase {} (order {})", p.name, p.order);
        } else {
            hazards += 1;
            for prob in problems {
                println!("HAZARD   phase {}: {prob}", p.name);
            }
        }
    }
    if hazards > 0 {
        eprintln!("-- {hazards} phase(s) with problems");
        process::exit(1);
    }
    println!("-- {} phase(s), well-formed and ordered", phases.len());
}

// ---- Receipts ledger (plan/0025) -------------------------------------------
//
// A receipt is the per-project, append-only, tool-written record of what the
// project did for one lifecycle phase: `done` (re-derivable evidence), `skip`
// (a cited reason), or `n-a` (tool-computed, never hand-asserted). `software
// --check` re-verifies each `done` by the manifest's closed `recheck =` command —
// never the receipt's own say-so (R1) — and HAZARDs a missing receipt, a stale
// one, a skipped protected core, or an `n-a` on a phase that applies.

struct Receipt {
    phase: String,
    component: Option<String>,
    disposition: String,
    evidence: Option<String>,
    reason: Option<String>,
    tool: Option<String>,
    recorded: Option<String>,
}

/// Parse `.host-receipts`: `[receipt "<phase>"]` or `[receipt "<phase>" "<component>"]`
/// stanzas (git-config style, mirrors `parse_software`'s `[build "s" "p"]`).
fn parse_receipts(text: &str) -> Vec<Receipt> {
    let mut out: Vec<Receipt> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(inner) = t.strip_prefix("[receipt \"").and_then(|r| r.strip_suffix("\"]")) {
            let (phase, component) = match inner.split_once("\" \"") {
                Some((p, c)) => (p.to_string(), Some(c.to_string())),
                None => (inner.to_string(), None),
            };
            out.push(Receipt {
                phase,
                component,
                disposition: String::new(),
                evidence: None,
                reason: None,
                tool: None,
                recorded: None,
            });
            continue;
        }
        let Some((key, val)) = t.split_once('=') else { continue };
        let (key, val) = (key.trim(), val.trim());
        let Some(cur) = out.last_mut() else { continue };
        match key {
            "disposition" => cur.disposition = val.to_string(),
            "evidence" => cur.evidence = Some(val.to_string()),
            "reason" => cur.reason = Some(val.to_string()),
            "tool" => cur.tool = Some(val.to_string()),
            "recorded" => cur.recorded = Some(val.to_string()),
            _ => {}
        }
    }
    out
}

/// The current receipt for (phase, component): the LAST matching stanza (append-only,
/// last-wins; earlier stanzas are retained history).
fn latest_receipt<'a>(receipts: &'a [Receipt], phase: &str, component: Option<&str>) -> Option<&'a Receipt> {
    receipts.iter().rev().find(|r| r.phase == phase && r.component.as_deref() == component)
}

fn receipt_stanza(r: &Receipt) -> String {
    let head = match &r.component {
        Some(c) => format!("[receipt \"{}\" \"{}\"]", r.phase, c),
        None => format!("[receipt \"{}\"]", r.phase),
    };
    let mut s = format!("{head}\n    disposition = {}\n", r.disposition);
    for (k, v) in [("evidence", &r.evidence), ("reason", &r.reason), ("tool", &r.tool), ("recorded", &r.recorded)] {
        if let Some(v) = v {
            s.push_str(&format!("    {k} = {v}\n"));
        }
    }
    s
}

/// Append a receipt to `.host-receipts` atomically (append-only; a blank line
/// separates stanzas). The tool is the only writer — Fen never hand-edits it.
/// A lifecycle phase whose receipt is a methodology-version event, the act of moving the
/// project to a template revision: `adopt` (sets the baseline) and `upgrade` (advances
/// it). These live in `.host-receipts`; every other phase host-lifecycle runs is
/// operational and lives in `.host-lifecycle-receipts` (plan/0037).
fn is_methodology_phase(phase: &str) -> bool {
    phase == "adopt" || phase == "upgrade"
}

/// The applied-set ids from BOTH layouts (plan/0037): the legacy `applied =` lines in
/// `.host` and the migrated ones in `.host-receipts`, unioned in first-seen order. The
/// gate reads an un-migrated, migrated, or transitional project alike.
fn read_applied_ids(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for f in [STAMP, RECEIPTS] {
        for id in stamp_values(&fs::read_to_string(root.join(f)).unwrap_or_default(), "applied") {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// The file the applied-set lives in: `.host` while a legacy `applied =` line is still in
/// the stamp, else `.host-receipts` (migrated, or a fresh adoption on this binary). New
/// applied lines and `--advance` compaction target it, so the set never fragments.
fn applied_file(root: &Path) -> &'static str {
    if stamp_values(&fs::read_to_string(root.join(STAMP)).unwrap_or_default(), "applied").is_empty() {
        RECEIPTS
    } else {
        STAMP
    }
}

/// Every lifecycle receipt from BOTH layouts (plan/0037): the methodology-version receipts
/// in `.host-receipts` and the operational ones in `.host-lifecycle-receipts`. The gate
/// unions them, so it re-checks an un-migrated, migrated, or transitional project.
fn read_all_receipts(root: &Path) -> Vec<Receipt> {
    let mut text = fs::read_to_string(root.join(RECEIPTS)).unwrap_or_default();
    if let Ok(op) = fs::read_to_string(root.join(LIFECYCLE_RECEIPTS)) {
        text.push('\n');
        text.push_str(&op);
    }
    parse_receipts(&text)
}

fn append_receipt(root: &Path, r: &Receipt) -> std::io::Result<()> {
    // Route by ontology (plan/0037): a methodology-version receipt (adopt/upgrade) to
    // `.host-receipts`, every operational receipt to `.host-lifecycle-receipts`.
    let path = root.join(if is_methodology_phase(&r.phase) { RECEIPTS } else { LIFECYCLE_RECEIPTS });
    let mut cur = fs::read_to_string(&path).unwrap_or_default();
    if !cur.is_empty() {
        if !cur.ends_with('\n') {
            cur.push('\n');
        }
        cur.push('\n');
    }
    cur.push_str(&receipt_stanza(r));
    write_atomic(&path, &cur)
}

/// Refuse a record that violates the receipt invariants (R1/R2). Pure + unit-tested:
/// `n-a` is tool-only, a `done` needs re-derivable evidence, a `skip` needs a reason
/// and a non-protected phase.
fn validate_receipt_record(name: &str, disposition: &str, evidence: Option<&str>, reason: Option<&str>, skippable: bool) -> Result<(), String> {
    match disposition {
        "n-a" => Err(format!("`n-a` is tool-computed from project state, never recorded by hand (phase `{name}`)")),
        "done" if evidence.is_none() => Err(format!("a `done` receipt needs `--evidence` (phase `{name}`); the evidence must be re-derivable")),
        "done" => Ok(()),
        "skip" if reason.is_none() => Err(format!("a `skip` receipt needs `--reason` (a `call/NNNN` for a substantive skip) (phase `{name}`)")),
        "skip" if !skippable => Err(format!("phase `{name}` is a protected core (`skippable = false`) and cannot be skipped")),
        "skip" => Ok(()),
        other => Err(format!("unknown disposition `{other}` — use `done` or `skip`")),
    }
}

/// The lifecycle the project is governed by, from the template `.host` records.
enum ManifestState {
    /// No `.host` — not adopted; no lifecycle gate.
    NotAdopted,
    /// `.host` present but the checked-out template carries no manifest (the adopted
    /// revision predates manifests) — the gate stays inert until the project upgrades.
    Absent,
    Live(Vec<Phase>),
}

/// Load the manifest from the template submodule (checked out at the adopted
/// revision, as `upgrade` already requires — same `find_template_dir` source as the
/// UPGRADING ledger). A template with no manifest is `Absent`, not an error, so a
/// pre-manifest adoption keeps passing ("nothing required until declared").
fn load_project_manifest(root: &Path) -> ManifestState {
    if !root.join(STAMP).is_file() {
        return ManifestState::NotAdopted;
    }
    match find_template_dir(root).and_then(|t| fs::read_to_string(t.join(MANIFEST)).ok()) {
        Some(text) => ManifestState::Live(parse_manifest(&text)),
        None => ManifestState::Absent,
    }
}

/// One gate line per (phase[,component]) the manifest declares.
struct GateLine {
    label: String,
    ok: bool,
    note: String,
    /// Run by `software_check`; a non-zero exit re-opens the `done` as a HAZARD (R1).
    recheck: Option<String>,
    /// The literal remediation command to copy when this is a HAZARD — the de-risk
    /// fold-back: never make a weak agent construct an exact command.
    remedy: Option<String>,
}

/// The pure receipt gate: one `GateLine` per (phase[,component]) the manifest
/// declares, given the project's receipts and capabilities. `has_where` = the
/// project has a software room; `components` = its software names (for
/// recurring-per-component phases). The recheck command is returned, not run, so
/// this stays pure and unit-testable; `software_check` executes it.
fn receipt_gate(phases: &[Phase], receipts: &[Receipt], has_where: bool, components: &[String]) -> Vec<GateLine> {
    let mut sorted: Vec<&Phase> = phases.iter().collect();
    sorted.sort_by_key(|p| p.order);
    let mut out = Vec::new();
    for p in sorted {
        let applies = match p.conditional_on() {
            Some("Where") => has_where,
            Some(_) => true, // unknown condition: fail-closed, assume it applies
            None => true,
        };
        let targets: Vec<Option<&str>> = if p.recurring() {
            components.iter().map(|c| Some(c.as_str())).collect()
        } else {
            vec![None]
        };
        if !applies || (p.recurring() && targets.is_empty()) {
            out.push(GateLine {
                label: format!("phase {}", p.name),
                ok: true,
                note: "n-a (does not apply to this project)".into(),
                recheck: None,
                remedy: None,
            });
            continue;
        }
        for comp in targets {
            let label = match comp {
                Some(c) => format!("phase {} ({c})", p.name),
                None => format!("phase {}", p.name),
            };
            let comp_arg = comp.map(|c| format!(" --component {c}")).unwrap_or_default();
            let remedy = format!("host-lifecycle receipt --record {}{comp_arg} --disposition done --evidence <...>", p.name);
            let line = match latest_receipt(receipts, &p.name, comp) {
                None => GateLine { label, ok: false, note: "no receipt".into(), recheck: None, remedy: Some(remedy) },
                Some(r) => match r.disposition.as_str() {
                    "done" => match &p.recheck {
                        Some(rc) => GateLine { label, ok: true, note: "done".into(), recheck: Some(rc.clone()), remedy: None },
                        None => GateLine { label, ok: false, note: "done but the manifest declares no `recheck =` — a done must be re-derivable (R1)".into(), recheck: None, remedy: None },
                    },
                    "skip" if !p.skippable => GateLine { label, ok: false, note: "skip on a protected core (`skippable = false`)".into(), recheck: None, remedy: None },
                    "skip" if r.reason.is_none() => GateLine { label, ok: false, note: "skip without a `reason`".into(), recheck: None, remedy: None },
                    "skip" => GateLine { label, ok: true, note: format!("skip ({})", r.reason.as_deref().unwrap_or("")), recheck: None, remedy: None },
                    "n-a" => GateLine { label, ok: false, note: "receipt asserts `n-a` but the phase applies (n-a is tool-computed, not recorded)".into(), recheck: None, remedy: None },
                    other => GateLine { label, ok: false, note: format!("unknown disposition `{other}`"), recheck: None, remedy: None },
                },
            };
            out.push(line);
        }
    }
    out
}

/// `migrate-receipts <dir>`: the tool-driven, idempotent re-homing (plan/0037). Move the
/// applied-set out of `.host` into `.host-receipts`, split the operational receipts out of
/// `.host-receipts` into `.host-lifecycle-receipts`, and leave the adopt/upgrade receipts in
/// `.host-receipts`. Writes atomically and reports what moved; a project already on the new
/// layout is a no-op. The agent runs one command and never hand-edits the files.
fn migrate_receipts(args: &[String]) {
    let dir = args.iter().find(|a| !a.starts_with("--")).map(String::as_str).unwrap_or(".");
    let root = match fs::canonicalize(Path::new(dir)) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    let is_applied_line = |l: &str| l.trim_start().strip_prefix("applied").is_some_and(|r| r.trim_start().starts_with('='));
    let stamp = fs::read_to_string(root.join(STAMP)).unwrap_or_default();
    let rc_text = fs::read_to_string(root.join(RECEIPTS)).unwrap_or_default();

    let receipts = parse_receipts(&rc_text);
    let has_operational = receipts.iter().any(|r| !is_methodology_phase(&r.phase));
    let stamp_has_applied = stamp.lines().any(&is_applied_line);
    if !stamp_has_applied && !has_operational {
        println!("migrate-receipts: already on the new layout; nothing to move");
        return;
    }

    // The full `applied =` lines (with recorded/via), from both files (transitional-safe),
    // de-duplicated by exact line: a partial prior run (a crash between writes) can leave the
    // same applied line in both files, and the recovery re-run must not double it (issue #9).
    let mut seen_applied = std::collections::HashSet::new();
    let applied_lines: Vec<String> = stamp
        .lines()
        .chain(rc_text.lines())
        .filter(|l| is_applied_line(l))
        .map(|l| l.trim().to_string())
        .filter(|l| seen_applied.insert(l.clone()))
        .collect();

    // `.host`: the stamp minus the applied lines.
    let mut new_stamp: String = stamp.lines().filter(|l| !is_applied_line(l)).collect::<Vec<_>>().join("\n");
    if !new_stamp.ends_with('\n') {
        new_stamp.push('\n');
    }

    // `.host-receipts`: the applied-set block, then the methodology-version receipts.
    let methodology: Vec<&Receipt> = receipts.iter().filter(|r| is_methodology_phase(&r.phase)).collect();
    let mut new_rc = String::new();
    if !applied_lines.is_empty() {
        new_rc.push_str(&applied_lines.join("\n"));
        new_rc.push('\n');
    }
    for r in &methodology {
        if !new_rc.is_empty() {
            new_rc.push('\n');
        }
        new_rc.push_str(&receipt_stanza(r));
    }

    // `.host-lifecycle-receipts`: existing content, then the operational receipts moved here.
    let operational: Vec<&Receipt> = receipts.iter().filter(|r| !is_methodology_phase(&r.phase)).collect();
    let mut new_op = fs::read_to_string(root.join(LIFECYCLE_RECEIPTS)).unwrap_or_default();
    for r in &operational {
        if !new_op.is_empty() {
            if !new_op.ends_with('\n') {
                new_op.push('\n');
            }
            new_op.push('\n');
        }
        new_op.push_str(&receipt_stanza(r));
    }

    // Crash-safety (issue #9): write each data-receiving file before the file that sheds that
    // data — applied lines move STAMP→RECEIPTS, operational receipts move RECEIPTS→
    // LIFECYCLE_RECEIPTS — so an I/O fault mid-set leaves the data duplicated (the re-run
    // dedups and converges) rather than stripped from the source before the destination has it.
    for (name, content) in [(LIFECYCLE_RECEIPTS, &new_op), (RECEIPTS, &new_rc), (STAMP, &new_stamp)] {
        if let Err(e) = write_atomic(&root.join(name), content) {
            eprintln!("host-lifecycle: cannot write {name}: {e}");
            process::exit(2);
        }
    }
    println!("migrate-receipts: moved {} applied line(s) into {RECEIPTS}; split {} operational receipt(s) to {LIFECYCLE_RECEIPTS}; kept {} methodology receipt(s) in {RECEIPTS}", applied_lines.len(), operational.len(), methodology.len());
}

fn receipt(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("--record") => receipt_record(&args[1..]),
        Some("--list") => receipt_list(args.get(1)),
        _ => {
            eprintln!("usage: host-lifecycle receipt --record <phase> [--component <c>] --disposition done|skip (--evidence <e> | --reason <r>) [<dir>]");
            eprintln!("       host-lifecycle receipt --list [<dir>]");
            process::exit(2);
        }
    }
}

fn receipt_record(args: &[String]) {
    let (mut phase, mut component, mut disposition, mut evidence, mut reason) = (None, None, None, None, None);
    let mut dir = String::from(".");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--component" => { component = args.get(i + 1).cloned(); i += 2; }
            "--disposition" => { disposition = args.get(i + 1).cloned(); i += 2; }
            "--evidence" => { evidence = args.get(i + 1).cloned(); i += 2; }
            "--reason" => { reason = args.get(i + 1).cloned(); i += 2; }
            s if s.starts_with("--") => { i += 1; }
            s if phase.is_none() => { phase = Some(s.to_string()); i += 1; }
            s => { dir = s.to_string(); i += 1; }
        }
    }
    let (Some(phase), Some(disposition)) = (phase, disposition) else {
        eprintln!("usage: host-lifecycle receipt --record <phase> --disposition done|skip (--evidence <e> | --reason <r>) [<dir>]");
        process::exit(2);
    };
    let root = Path::new(&dir);
    // Protectedness comes from the manifest when one is live; a phase the manifest
    // does not declare is refused (you cannot receipt a phase outside the lifecycle).
    let skippable = match load_project_manifest(root) {
        ManifestState::Live(ps) => match ps.iter().find(|p| p.name == phase) {
            Some(p) => p.skippable,
            None => {
                eprintln!("host-lifecycle: `{phase}` is not a phase in the lifecycle manifest");
                process::exit(2);
            }
        },
        _ => true,
    };
    if let Err(e) = validate_receipt_record(&phase, &disposition, evidence.as_deref(), reason.as_deref(), skippable) {
        eprintln!("host-lifecycle: {e}");
        process::exit(2);
    }
    let r = Receipt {
        phase: phase.clone(),
        component: component.clone(),
        disposition: disposition.clone(),
        evidence,
        reason,
        tool: Some(format!("host-lifecycle@{}", env!("CARGO_PKG_VERSION"))),
        recorded: Some(today()),
    };
    if let Err(e) = append_receipt(root, &r) {
        eprintln!("host-lifecycle: cannot write {RECEIPTS}: {e}");
        process::exit(2);
    }
    let comp = component.map(|c| format!(" ({c})")).unwrap_or_default();
    println!("recorded receipt: phase {phase}{comp} = {disposition}");
}

fn receipt_list(dir: Option<&String>) {
    let root = Path::new(dir.map_or(".", |s| s.as_str()));
    let receipts = read_all_receipts(root);
    if receipts.is_empty() {
        println!("no receipts recorded");
        return;
    }
    let mut seen: Vec<(String, Option<String>)> = Vec::new();
    let mut lines = Vec::new();
    for r in receipts.iter().rev() {
        let key = (r.phase.clone(), r.component.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let comp = r.component.as_deref().map(|c| format!(" ({c})")).unwrap_or_default();
        let detail = r.evidence.as_deref().or(r.reason.as_deref()).unwrap_or("");
        lines.push(format!("{:<6} {}{comp}  {detail}", r.disposition, r.phase));
    }
    lines.reverse();
    for l in lines {
        println!("{l}");
    }
}

// ---- Release orchestration (plan/0025) -------------------------------------
//
// `host-lifecycle release <component>` is the single agent-facing driver (Fen
// fold-back #1: one command, the tool holds the sequence). It gates each step and
// COMPUTES the version and artifact hash itself, so a weak agent never names a semver
// level (fold-back #2) and never hand-derives a hash (the strong-agent near-miss this
// milestone exists to prevent). The migrated escape prints and validates an exact
// `call/NNNN` skip citation (fold-back #3). The outward `git push` is never run by the
// tool — software-first ordering and the push-authorization rule keep it operator-run.

/// The change class the agent answers — the ONE release decision Fen makes. Concrete
/// (does this remove/rename a flag or change output? add one? neither?), never the
/// semver level: the Fen de-risk had the 4B reason "breaking" yet answer `minor`, so
/// the tool maps the class to a level and the agent never picks the level.
enum ChangeClass {
    Breaking,
    Feature,
    Fix,
}

const CHANGE_CLASSES: &str = "removes-flag|adds-flag|neither";

fn parse_change_class(s: &str) -> Option<ChangeClass> {
    match s {
        "removes-flag" | "renames-flag" | "changes-output" | "breaking" => Some(ChangeClass::Breaking),
        "adds-flag" | "feature" => Some(ChangeClass::Feature),
        "neither" | "fix" => Some(ChangeClass::Fix),
        _ => None,
    }
}

/// Map a change class to the next version from the current `x.y.z`. Pre-1.0 (`0.y.z`)
/// the minor is cargo's compat position, so a breaking change bumps it (never jumping
/// 0.4.2 to 1.0.0); a feature also bumps the minor — the convention this tool family
/// itself follows (each `host-lifecycle 0.N.0` was a feature) and the visibility a 0.x
/// project wants — and only a fix is a patch. Post-1.0 it is textbook semver. The tool
/// writes this; the agent never types a version.
fn next_version(current: &str, class: &ChangeClass) -> Result<String, String> {
    let parts: Vec<&str> = current.trim().trim_start_matches('v').split('.').collect();
    if parts.len() != 3 {
        return Err(format!("version `{current}` is not `x.y.z`"));
    }
    let mut n = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        n[i] = p.parse().map_err(|_| format!("version `{current}` has a non-numeric component `{p}`"))?;
    }
    let (major, minor, patch) = (n[0], n[1], n[2]);
    let (a, b, c) = if major == 0 {
        match class {
            ChangeClass::Breaking | ChangeClass::Feature => (0, minor + 1, 0),
            ChangeClass::Fix => (0, minor, patch + 1),
        }
    } else {
        match class {
            ChangeClass::Breaking => (major + 1, 0, 0),
            ChangeClass::Feature => (major, minor + 1, 0),
            ChangeClass::Fix => (major, minor, patch + 1),
        }
    };
    Ok(format!("{a}.{b}.{c}"))
}

/// Replace the `version = "…"` line in the `[package]` table of a Cargo.toml. Returns
/// `None` if there is no `[package]` version line (a tool with no manifest never reaches
/// this). Only the first `[package]` version is touched — dependency versions in other
/// tables are left alone.
fn set_cargo_version(text: &str, new: &str) -> Option<String> {
    // Bump `[package] version` if the manifest has one, else `[workspace.package] version` (a virtual
    // workspace root, e.g. host-reference), so a workspace component bumps through the same path.
    let target = if has_section_version(text, "[package]") {
        "[package]"
    } else {
        "[workspace.package]"
    };
    let mut in_target = false;
    let mut done = false;
    let mut out = String::with_capacity(text.len() + 16);
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_target = t == target;
        }
        if in_target && !done {
            if let Some(rest) = t.strip_prefix("version") {
                if rest.trim_start().starts_with('=') {
                    out.push_str(&format!("version = \"{new}\"\n"));
                    done = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    done.then_some(out)
}

/// Whether a Cargo.toml has a literal `version = "..."` under the given section header.
fn has_section_version(text: &str, header: &str) -> bool {
    let mut in_section = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t == header;
        }
        if in_section {
            if let Some(rest) = t.strip_prefix("version") {
                if rest.trim_start().starts_with('=') {
                    return true;
                }
            }
        }
    }
    false
}

/// A migrated-escape skip cites a bare `call/NNNN` — never a phase name or a
/// `phase/NNNN` token. The Fen de-risk emitted `--skip reproducible-build/0031`,
/// conflating the phase with the decision id, so `--record` rejects anything but
/// `call/` + digits and `--next` prints the literal correct command to copy.
fn valid_skip_citation(s: &str) -> bool {
    matches!(s.strip_prefix("call/"), Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// An artifact-bearing component (a recorded `artifact` hash, e.g. host-lint) re-builds
/// and re-hashes on release; a tool (skills/scripts, no artifact, e.g. host-prove) is a
/// tag-only release. Read from the recipe, not guessed (plan/0025: "the orchestration
/// reads the recipe; it is not one fixed procedure").
fn is_artifact_bearing(s: &Software) -> bool {
    s.builds_view().iter().any(|b| b.artifact.is_some())
}

/// A component's `repro-exempt` citation (migrated/foreign provenance), if any — the
/// only components for which a `release --skip` is reachable (R3: never greenfield).
fn repro_exempt_cite(s: &Software) -> Option<String> {
    s.builds_view().iter().find_map(|b| b.repro_exempt.map(String::from))
}

fn release(args: &[String]) {
    if args.first().map(String::as_str) == Some("--record") {
        release_record_skip(&args[1..]);
        return;
    }
    let (mut component, mut change_class, mut dir, mut preview) = (None, None, String::from("."), false);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--next" | "--preview" => { preview = true; i += 1; }
            "--change-class" => { change_class = args.get(i + 1).cloned(); i += 2; }
            s if s.starts_with("--") => { i += 1; }
            s if component.is_none() => { component = Some(s.to_string()); i += 1; }
            s => { dir = s.to_string(); i += 1; }
        }
    }
    let Some(component) = component else {
        eprintln!("usage: host-lifecycle release <component> [--change-class {CHANGE_CLASSES}] [<dir>]");
        eprintln!("       host-lifecycle release --next <component> [<dir>]");
        eprintln!("       host-lifecycle release --record <component> --skip call/NNNN [<dir>]");
        process::exit(2);
    };
    run_release(Path::new(&dir), &component, change_class.as_deref(), preview);
}

/// Resolve the component's recipe + a live `release` phase, exiting with a clear
/// message when either is missing (release is a manifest-driven phase).
fn release_context<'a>(root: &Path, recipe: &'a [Software], component: &str) -> &'a Software {
    let Some(s) = recipe.iter().find(|s| s.name == component) else {
        eprintln!("host-lifecycle: no component `{component}` in {SOFTWARE}");
        process::exit(2);
    };
    match load_project_manifest(root) {
        ManifestState::Live(ps) if ps.iter().any(|p| p.name == "release") => {}
        ManifestState::Live(_) => {
            eprintln!("host-lifecycle: the lifecycle manifest declares no `release` phase (upgrade the template — plan/0025 spine)");
            process::exit(2);
        }
        _ => {
            eprintln!("host-lifecycle: release needs a {MANIFEST} with a `release` phase; the adopted template has none (adopt/upgrade — plan/0025 spine)");
            process::exit(2);
        }
    }
    s
}

/// The current version of a component: an artifact-bearing crate reads its worktree
/// `Cargo.toml` `[package] version`; a tool reads its latest `v*` git tag.
fn current_version(root: &Path, s: &Software) -> Result<String, String> {
    let work = worktree_dir(root, &s.name, &s.branch);
    // The current version is the last RELEASE TAG — the canonical released version —
    // never the worktree Cargo.toml. The release bumps Cargo.toml in place, so reading it
    // back would drift the version on a re-run (each attempt would re-bump from the last).
    // A never-tagged component falls back to its declared Cargo.toml version (an artifact
    // crate, so its first release starts from the version it ships) or 0.0.0 (a tag-only
    // tool); the first release computes from the change-class either way (plan/0029).
    if let Some(tag) = git_out(&work, &["describe", "--tags", "--abbrev=0"]) {
        let v = tag.trim().trim_start_matches('v');
        if !v.is_empty() {
            return Ok(v.to_string());
        }
    }
    if is_artifact_bearing(s) {
        let toml = fs::read_to_string(work.join("Cargo.toml")).map_err(|e| format!("cannot read {}/Cargo.toml: {e}", s.name))?;
        cargo_version(&toml).ok_or_else(|| format!("{}/Cargo.toml has no [package] version", s.name))
    } else {
        Ok("0.0.0".to_string())
    }
}

/// Read the version from a Cargo.toml (the inverse of `set_cargo_version`). `[package] version` is
/// preferred; a virtual workspace (a root with no `[package]`) keeps the version in
/// `[workspace.package]`, so fall back to that, which lets a workspace component release through the
/// same path as a single-crate one.
fn cargo_version(text: &str) -> Option<String> {
    let mut package = None;
    let mut workspace_package = None;
    let mut section = "";
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            section = match t {
                "[package]" => "package",
                "[workspace.package]" => "workspace",
                _ => "",
            };
        }
        if !section.is_empty() {
            if let Some(rest) = t.strip_prefix("version") {
                if let Some(v) = rest.trim_start().strip_prefix('=') {
                    let value = v.trim().trim_matches('"').to_string();
                    if section == "package" {
                        package = Some(value);
                    } else {
                        workspace_package = Some(value);
                    }
                }
            }
        }
    }
    package.or(workspace_package)
}

/// Read the `[package] name` from a Cargo.toml — the crate name, which keys the crate's
/// own `[[package]]` block in Cargo.lock (may differ from the component name).
fn cargo_package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
        }
        if in_package {
            if let Some(rest) = t.strip_prefix("name") {
                if let Some(v) = rest.trim_start().strip_prefix('=') {
                    return Some(v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    None
}

/// Sync a crate's own version line in Cargo.lock to `new`, so a `--locked` build does not
/// fail on a stale lock after the Cargo.toml bump. Targeted text edit of the `[[package]]`
/// block whose `name` matches the crate — no cargo invocation, no network. Returns the
/// updated text, or None if the crate's block (or version line) was not found.
fn set_lock_version(lock: &str, crate_name: &str, new: &str) -> Option<String> {
    let name_line = format!("name = \"{crate_name}\"");
    let mut out = String::with_capacity(lock.len() + 8);
    let mut in_target = false;
    let mut bumped = false;
    for line in lock.lines() {
        let t = line.trim();
        if t == "[[package]]" {
            in_target = false;
        } else if t == name_line {
            in_target = true;
        }
        if in_target && !bumped && t.starts_with("version =") {
            out.push_str(&format!("version = \"{new}\"\n"));
            in_target = false;
            bumped = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    bumped.then_some(out)
}

/// Parse the `[workspace] members = [...]` array into member directory paths. Handles a single-line or
/// multi-line array; the paths carry no brackets, so the first `]` after `members` closes the array.
fn workspace_members(root_toml: &str) -> Vec<String> {
    let Some(start) = root_toml.find("members") else { return Vec::new() };
    let region = &root_toml[start..];
    let (Some(a), Some(b)) = (region.find('['), region.find(']')) else { return Vec::new() };
    if b <= a {
        return Vec::new();
    }
    region[a + 1..b]
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect()
}

/// Whether a member crate's `[package]` inherits the workspace version (`version.workspace = true`),
/// so it moves with the `[workspace.package]` bump and its `Cargo.lock` entry must move too.
fn inherits_workspace_version(member_toml: &str) -> bool {
    let mut in_package = false;
    for line in member_toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
        }
        if in_package {
            let c = t.replace(' ', "");
            if c.starts_with("version.workspace=true") || c.starts_with("version={workspace=true") {
                return true;
            }
        }
    }
    false
}

fn run_release(root: &Path, component: &str, change_class: Option<&str>, preview: bool) {
    let recipe = load_software(root);
    let s = release_context(root, &recipe, component);
    let work = worktree_dir(root, &s.name, &s.branch);

    // The migrated escape: a repro-exempt component cannot reproduce a foreign
    // toolchain, so its release is a cited skip — the tool prints the LITERAL command
    // with the real citation, so Fen copies an exact `call/NNNN` (fold-back #3).
    if let Some(cite) = repro_exempt_cite(s) {
        println!("release {component} is repro-exempt ({cite}) — record the migrated skip:");
        println!("    host-lifecycle release --record {component} --skip {cite}");
        return;
    }

    if preview {
        match change_class {
            None => {
                println!("next: host-lifecycle release {component} --change-class <{CHANGE_CLASSES}>");
                println!("  (the tool maps the change class to the version; you never name a semver level)");
            }
            Some(_) => println!("next: host-lifecycle release {component} --change-class {} (runs the gated sequence)", change_class.unwrap()),
        }
        return;
    }

    // Step 1 — verify. Run the manifest `verify` phase's closed recheck; red blocks.
    if let Some(cmd) = verify_recheck(root) {
        println!("release {component}: running the verify gate …");
        if !run_verify(root, &cmd) {
            eprintln!("host-lifecycle: verify is RED — release blocked. Fix the verify sweep, then re-run.");
            process::exit(1);
        }
        println!("  verify: green");
    }

    // Step 1b (plan/0069) — discharge the released component's obligations offline. A
    // release must not ship its own component red on a local lane: run the obligations
    // check (no --tests, no --rederive) for each .allium the component carries, and
    // block on MISSING/STALE dispositions or STALE input digests (the born-red-tag
    // class, plan/0045/plan/0048/host-lint v0.14.0). Test-name resolution and proof
    // re-derivation stay in component CI; allium-cli must be on PATH (plan/0069 Q1).
    for spec_path in find_allium_specs(&work) {
        let rel = spec_path.strip_prefix(&work).unwrap_or(&spec_path);
        println!("release {component}: discharging {} …", rel.display());
        let manifest = spec_path.with_extension("obligations");
        let problems = match discharge_problems(&spec_path, &manifest) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("host-lifecycle: {component} discharge error: {e} — release blocked");
                process::exit(1);
            }
        };
        if !problems.is_empty() {
            for p in &problems {
                eprintln!("    {p}");
            }
            eprintln!(
                "host-lifecycle: {component} obligations discharge has {} problem(s) — release blocked. Fix the discharge, then re-run.",
                problems.len()
            );
            process::exit(1);
        }
    }

    // Step 2 — change class. The one decision the agent supplies; the tool maps it.
    let Some(class_str) = change_class else {
        println!("next: host-lifecycle release {component} --change-class <{CHANGE_CLASSES}>");
        println!("  removes-flag = a removed/renamed public flag or changed output (breaking)");
        println!("  adds-flag    = a new flag or behaviour (feature)");
        println!("  neither      = a fix only");
        return;
    };
    let Some(class) = parse_change_class(class_str) else {
        eprintln!("host-lifecycle: unknown change class `{class_str}` — use one of {CHANGE_CLASSES}");
        process::exit(2);
    };
    let cur = match current_version(root, s) {
        Ok(v) => v,
        Err(e) => { eprintln!("host-lifecycle: {e}"); process::exit(2); }
    };
    let new = match next_version(&cur, &class) {
        Ok(v) => v,
        Err(e) => { eprintln!("host-lifecycle: {e}"); process::exit(2); }
    };
    println!("  version: {cur} -> {new} (the tool computed this from the change class)");

    if !is_artifact_bearing(s) {
        // A tool: tag-only, no artifact to rebuild/re-hash.
        println!("\nrelease {component} v{new} — tool (no artifact). Outward steps (operator-run):");
        println!("    cd {} && git tag -a v{new} -m 'release v{new}' && git push origin v{new}", work.display());
        println!("    host-lifecycle receipt --record release --component {component} --disposition done --evidence v{new}");
        return;
    }

    // Step 3 — build in the recorded toolchain and COMPUTE the canonical hash. With no
    // container runtime the release BLOCKS — the re-pin/re-hash hazard is never handed
    // to a weak agent to do by hand (R5/R6).
    let Some(view) = s.builds_view().into_iter().find(|b| b.artifact.is_some()) else {
        eprintln!("host-lifecycle: {component} has no artifact build to release");
        process::exit(2);
    };
    let (artifact_path, _old_hash) = view.artifact.expect("filtered to artifact-bearing");
    let Some(image) = view.toolchain else {
        eprintln!("host-lifecycle: {component} records an artifact but no `toolchain` image — cannot build the canonical hash in a pinned environment");
        process::exit(1);
    };
    let Some(runtime) = container_runtime() else {
        eprintln!("host-lifecycle: no container runtime (docker/podman) — release BLOCKS; the canonical hash must come from the recorded toolchain {image}, never an ambient build (R5/R6)");
        process::exit(1);
    };

    // First, bump the worktree Cargo.toml — the tool writes the version (fold-back #2).
    let toml_path = work.join("Cargo.toml");
    let toml = match fs::read_to_string(&toml_path) {
        Ok(t) => t,
        Err(e) => { eprintln!("host-lifecycle: cannot read {}: {e}", toml_path.display()); process::exit(2); }
    };
    if cargo_version(&toml).as_deref() != Some(&new) {
        match set_cargo_version(&toml, &new) {
            Some(updated) => {
                if let Err(e) = fs::write(&toml_path, updated) {
                    eprintln!("host-lifecycle: cannot write {}: {e}", toml_path.display());
                    process::exit(2);
                }
                println!("  bumped {}/Cargo.toml to {new}", s.name);
            }
            None => { eprintln!("host-lifecycle: {}/Cargo.toml has no [package] version to bump", s.name); process::exit(2); }
        }
    }

    // Sync Cargo.lock's own-version so the pinned `--locked` build does not fail on a
    // stale lock (the bump changes the crate's version, which the lock records too).
    let lock_path = work.join("Cargo.lock");
    if let Ok(lock) = fs::read_to_string(&lock_path) {
        // A single crate bumps just itself; a virtual workspace bumps every member that inherits the
        // workspace version, since the [workspace.package] bump moves them all and a stale member
        // version fails the pinned `--locked` build.
        let is_workspace =
            !has_section_version(&toml, "[package]") && has_section_version(&toml, "[workspace.package]");
        let crates: Vec<String> = if is_workspace {
            workspace_members(&toml)
                .into_iter()
                .filter_map(|m| fs::read_to_string(work.join(&m).join("Cargo.toml")).ok())
                .filter(|t| inherits_workspace_version(t))
                .filter_map(|t| cargo_package_name(&t))
                .collect()
        } else {
            vec![cargo_package_name(&toml).unwrap_or_else(|| s.name.clone())]
        };
        let mut updated = lock.clone();
        for crate_name in &crates {
            if let Some(u) = set_lock_version(&updated, crate_name, &new) {
                updated = u;
            }
        }
        if updated != lock {
            if let Err(e) = fs::write(&lock_path, updated) {
                eprintln!("host-lifecycle: cannot write {}: {e}", lock_path.display());
                process::exit(2);
            }
            println!("  synced {}/Cargo.lock to {new} ({} crate(s))", s.name, crates.len());
        }
    }

    // plan/0032: a pinned dependency bundle makes the release build hermetic. Stage it
    // into the canonical worktree and build under `--network none`. Because this is the
    // live worktree (not a throwaway), snapshot the `.cargo/config.toml` and remove the
    // staged tree afterward, so the release commit carries only the version bump.
    let offline = s.deps_bundle.is_some();
    let cfg_path = work.join(".cargo/config.toml");
    let mut deps_guard = None;
    if let Some((url, want)) = &s.deps_bundle {
        let cfg_backup = fs::read_to_string(&cfg_path).ok();
        println!("  staging deps-bundle (verifying recorded sha) …");
        if let Err(e) = stage_deps_bundle(&work, url, want) {
            eprintln!("host-lifecycle: {e} — release blocked");
            process::exit(1);
        }
        // The guard reverts the staged vendor dir + config edit on Drop, so an abnormal exit
        // (a panic) before the explicit restore below still leaves only the version bump (#22).
        deps_guard = Some(StagedDepsGuard { work: work.clone(), cfg_backup, armed: true });
    }

    // Build the bumped worktree in the recorded image and hash the artifact. The hash
    // is computed from this verified build — the tool refuses to record any other.
    println!("  building {component} in {image} to compute the canonical hash …");
    let build_ok = run_build_in_container(runtime, image, view.build.unwrap_or("cargo build --release"), &work, offline);

    // Restore the worktree (disarming the guard so the revert is not repeated on drop): drop the
    // staged vendor dir and revert the config edit, so a following `git commit -am` carries only
    // the version bump, never the source-replacement.
    if let Some(g) = deps_guard.as_mut() {
        g.restore();
    }
    if !build_ok {
        eprintln!("host-lifecycle: build failed in {runtime} {image} — release blocked");
        process::exit(1);
    }
    let Some(hash) = sha256_file(&work.join(artifact_path)) else {
        eprintln!("host-lifecycle: built artifact {artifact_path} not found after the build");
        process::exit(1);
    };
    println!("  canonical hash: {hash}");

    // Step 4 — the operator-run outward sequence, with the tool-computed values filled
    // in. The tool never pushes (software-first ordering + push authorization); it
    // hands over the EXACT re-pin/re-hash so nothing is hand-derived.
    println!("\nrelease {component} v{new} — verified build reproduces. Outward steps (operator-run):");
    println!("    cd {} && git commit -am 'release v{new}' && git push", work.display());
    println!("    git -C {} tag -a v{new} -m 'release v{new}' && git -C {0} push origin v{new}", work.display());
    println!("  then re-pin {SOFTWARE} for [software \"{component}\"]:");
    println!("    pin      = <the pushed commit SHA>");
    println!("    artifact = {artifact_path} {hash}");
    let carried = carried_template_tools(root, &recipe);
    for line in template_pin_bump_lines(component, &new, &carried) {
        println!("{line}");
    }
    println!("    host-lifecycle receipt --record release --component {component} --disposition done --evidence v{new}@{hash}");
}

/// The outward template-pin-bump steps a release must run (call/0038): a host-lifecycle release
/// is incomplete until host-template's prose-CI pin equals the released commit, or `software
/// --check` HAZARDs the drift. Empty for any other component, since only host-lifecycle is pinned
/// by the template today. Pure, so the exact steps are unit-tested.
/// The host-template carried-pin bump steps a release must run (call/0038). `carried` is
/// the derived set (`carried_template_tools`: `.host-software` ∩ `tools/`); a release of any
/// carried tool must bump its `tools/<component>` submodule, or `software --check` HAZARDs
/// the drift. host-lifecycle is also pinned via the prose-CI `--rev` install (the prose-gate
/// tool), so it carries an extra step. The carried set is passed in (derived by the caller),
/// keeping this pure and unit-testable; `template_pin_problems` consumes the same set, so the
/// detection and the prompt cannot skew. plan/0069.
fn template_pin_bump_lines(component: &str, new: &str, carried: &[String]) -> Vec<String> {
    if !carried.iter().any(|c| c == component) {
        return Vec::new();
    }
    let subpath = format!("tools/{component}");
    let mut lines = vec![format!("  then bump host-template's pins for {component} (call/0038):")];
    if component == "host-lifecycle" {
        lines.push(format!(
            "    set --rev in host-template/.github/workflows/prose.yml to <the pushed commit SHA> (and the v{new} comment)"
        ));
    }
    lines.push(format!(
        "    cd host-template && git submodule update --init {subpath} && git -C {subpath} fetch origin && git -C {subpath} checkout <the pushed commit SHA> && git add {subpath} && git commit -am 'pin {component} v{new}' && git push && cd .."
    ));
    lines.push("    git add host-template && git commit -m 'bump host-template pointer' && git push".to_string());
    lines
}

/// The `verify` phase's closed `recheck` command from the live manifest (the verify
/// sweep the release gate runs). `None` when no manifest verify phase declares one.
fn verify_recheck(root: &Path) -> Option<String> {
    match load_project_manifest(root) {
        ManifestState::Live(ps) => ps.into_iter().find(|p| p.name == "verify").and_then(|p| p.recheck),
        _ => None,
    }
}

/// `release --record <component> --skip call/NNNN`: the content-validated migrated
/// escape (R3). The citation must be a bare `call/NNNN` (fold-back #3); the cited
/// decision must exist, be accepted, and be scoped (not the methodology meta-decision);
/// and the component must be repro-exempt (never greenfield-reachable).
fn release_record_skip(args: &[String]) {
    let (mut component, mut cite, mut dir) = (None, None, String::from("."));
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--skip" => { cite = args.get(i + 1).cloned(); i += 2; }
            s if s.starts_with("--") => { i += 1; }
            s if component.is_none() => { component = Some(s.to_string()); i += 1; }
            s => { dir = s.to_string(); i += 1; }
        }
    }
    let (Some(component), Some(cite)) = (component, cite) else {
        eprintln!("usage: host-lifecycle release --record <component> --skip call/NNNN [<dir>]");
        process::exit(2);
    };
    let root = Path::new(&dir);
    if !valid_skip_citation(&cite) {
        eprintln!("host-lifecycle: `--skip {cite}` is not a bare `call/NNNN` — cite the decision id, not a phase name");
        process::exit(2);
    }
    let recipe = load_software(root);
    let s = release_context(root, &recipe, &component);
    if repro_exempt_cite(s).is_none() {
        eprintln!("host-lifecycle: {component} is not repro-exempt — a release skip is only for migrated/foreign provenance, never a greenfield component (R3)");
        process::exit(2);
    }
    if !cited_decision_exists(root, &cite) {
        eprintln!("host-lifecycle: cited decision {cite} not found under call/");
        process::exit(2);
    }
    let body = decision_body(root, &cite).unwrap_or_default();
    if let Some(problem) = decision_scope_problem(&body) {
        eprintln!("host-lifecycle: {cite} cannot authorize a skip: {problem}");
        process::exit(2);
    }
    if decision_field(&body, "Status").map(|s| s.to_ascii_lowercase()).is_none_or(|st| !st.starts_with("accepted")) {
        eprintln!("host-lifecycle: {cite} is not `Status: accepted` — only an accepted decision authorizes a skip");
        process::exit(2);
    }
    let r = Receipt {
        phase: "release".to_string(),
        component: Some(component.clone()),
        disposition: "skip".to_string(),
        evidence: None,
        reason: Some(cite.clone()),
        tool: Some(format!("host-lifecycle@{}", env!("CARGO_PKG_VERSION"))),
        recorded: Some(today()),
    };
    if let Err(e) = append_receipt(root, &r) {
        eprintln!("host-lifecycle: cannot write {RECEIPTS}: {e}");
        process::exit(2);
    }
    println!("recorded receipt: phase release ({component}) = skip ({cite})");
}

/// Read a cited decision's markdown body (the first `call/<num>-*.md` matching the id).
fn decision_body(root: &Path, cite: &str) -> Option<String> {
    let num = cite.trim_start_matches("call/").split('-').next().unwrap_or("");
    if num.is_empty() {
        return None;
    }
    let pfx = format!("{num}-");
    let rd = fs::read_dir(root.join("call")).ok()?;
    for e in rd.filter_map(|e| e.ok()) {
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with(&pfx) && n.ends_with(".md") {
            return fs::read_to_string(e.path()).ok();
        }
    }
    None
}

#[cfg(test)]
mod remap_tests {
    use super::*;

    fn rule(old: &str, new: &str) -> Rule {
        Rule { old_lc: old.to_ascii_lowercase(), new: new.to_string() }
    }

    #[test]
    fn word_bounded_and_case_insensitive() {
        let r = vec![rule("phase 5", "mcp-integration")];
        assert_eq!(apply_rules("Phase 5 done", &r), "mcp-integration done");
        assert_eq!(apply_rules("PHASE 5.", &r), "mcp-integration.");
        // boundaries: a longer numeral or a glued letter is a different token
        assert_eq!(apply_rules("phase 50 done", &r), "phase 50 done");
        assert_eq!(apply_rules("rephase 5", &r), "rephase 5");
    }

    #[test]
    fn longest_match_first_avoids_clobber() {
        let mut r = vec![rule("phase 5", "mcp"), rule("phase 5.0", "bringup")];
        r.sort_by_key(|x| std::cmp::Reverse(x.old_lc.len()));
        assert_eq!(apply_rules("Phase 5.0 and Phase 5", &r), "bringup and mcp");
    }

    #[test]
    fn preserves_line_structure() {
        let r = vec![rule("phase 4", "command-execution")];
        assert_eq!(apply_text("a Phase 4\nb\n", &r, false), "a command-execution\nb\n");
        assert_eq!(apply_text("Phase 4", &r, false), "command-execution");
        assert_eq!(apply_text("x\r\nPhase 4\r\n", &r, false), "x\r\ncommand-execution\r\n");
    }

    // host-lifecycle#12: remap --apply must not rewrite inside a `host-lint:ignore`
    // fence (markdown), the box the naming scan already skips, or it corrupts the very
    // verbatim record the box preserves. Outside it (and on a regular code fence) the
    // rule still applies.
    #[test]
    fn apply_text_skips_host_lint_ignore_fence() {
        let r = vec![rule("phase 4", "command-execution")];
        let md = "outside Phase 4\n```host-lint:ignore\nfrozen Phase 4 citation\n```\nafter Phase 4\n";
        let got = apply_text(md, &r, true);
        assert!(got.contains("frozen Phase 4 citation"), "fenced tell preserved: {got}");
        assert!(got.contains("outside command-execution"), "before the fence substituted: {got}");
        assert!(got.contains("after command-execution"), "after the fence substituted: {got}");
        assert!(!apply_text(md, &r, false).contains("frozen Phase 4 citation"), "non-md: fence not honored");
    }

    // host-lifecycle#13: remap's sanctioned vocabulary is host-lint's LEXICON (canonical),
    // with the legacy `.host-lint-allow` merged in as a deprecated alias, so the two never
    // drift into a token that one honours and the other flags.
    #[test]
    fn remap_allow_unifies_lexicon_and_legacy() {
        let base = std::env::temp_dir().join(format!("hl-remapallow-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("LEXICON"), "sanctioned widget 5\n").unwrap();
        fs::write(base.join(ALLOW), "legacy token 3\n").unwrap();
        let allow = remap_allow(&base);
        assert!(allow.iter().any(|p| p == "sanctioned widget 5"), "LEXICON phrase is honoured (single source): {allow:?}");
        assert!(allow.iter().any(|p| p == "legacy token 3"), "legacy .host-lint-allow merged as an alias: {allow:?}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn unmapped_tokens_are_never_touched() {
        // Only the mapped milestone name changes; an unmapped code stays verbatim.
        let r = vec![rule("phase 5", "mcp-integration")];
        assert_eq!(
            apply_rules("Phase 5.3 weed #1 finding #7", &r),
            "mcp-integration.3 weed #1 finding #7"
        );
    }

    // issue #7: an absent `.host-remap` is a fail-safe no-op (empty rule set), not an error.
    #[test]
    fn load_remap_absent_is_empty() {
        let base = std::env::temp_dir().join(format!("hl-remap-absent-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        assert!(load_remap(&base).is_empty(), "a missing .host-remap yields no rules, not an exit");
        let _ = fs::remove_dir_all(&base);
    }

    // issue #7: a present-but-blank (or comments-only) `.host-remap` is also an empty rule set —
    // the same fail-safe no-op as an absent one, distinct from a malformed line which still errors.
    #[test]
    fn load_remap_blank_or_comments_only_is_empty() {
        let base = std::env::temp_dir().join(format!("hl-remap-blank-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join(REMAP), "\n# only a comment\n\n").unwrap();
        assert!(load_remap(&base).is_empty(), "a blank/comments-only .host-remap yields no rules");
        let _ = fs::remove_dir_all(&base);
    }

    // issue #7: an empty dictionary renames nothing and writes nothing, without demanding a
    // clean git tree (the no-op returns before the clean-tree guard).
    #[test]
    fn empty_apply_is_a_noop_that_writes_nothing() {
        let base = std::env::temp_dir().join(format!("hl-remap-noop-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("doc.md"), "a Phase 2 note\n").unwrap();
        remap_apply(&base, &[], &[], false);
        assert_eq!(fs::read_to_string(base.join("doc.md")).unwrap(), "a Phase 2 note\n", "no rules -> file untouched");
        let _ = fs::remove_dir_all(&base);
    }

    // issue #7 anti-hollow guard: `--check` with zero rules must still scan every target and
    // report a tell, so an empty (or absent) dictionary never yields a clean verdict without a
    // scan. A clean tree with zero rules still exits 0.
    #[test]
    fn check_over_empty_rules_still_audits() {
        let base = std::env::temp_dir().join(format!("hl-remap-audit-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("tell.md"), "# Phase 2\n\nthe next phase begins\n").unwrap();
        assert_ne!(
            remap_check_code(&base, &[], &[], &[]),
            0,
            "zero rules must still surface the phase tell (anti-hollow)"
        );
        fs::remove_file(base.join("tell.md")).unwrap();
        fs::write(base.join("clean.md"), "the cat sat on the mat\n").unwrap();
        assert_eq!(
            remap_check_code(&base, &[], &[], &[]),
            0,
            "zero rules over a clean tree is a clean verdict"
        );
        let _ = fs::remove_dir_all(&base);
    }

    // issue #24: collect_files takes the entry type from the DirEntry (no symlink following)
    // and skips symlinks, like the sibling tracked_markdown — a symlinked file is not collected
    // and a symlinked directory is not descended.
    #[cfg(unix)]
    #[test]
    fn collect_files_skips_symlinks() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("hl-collect-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("realdir")).unwrap();
        fs::write(base.join("real.md"), "x").unwrap();
        fs::write(base.join("target.md"), "y").unwrap();
        fs::write(base.join("realdir/inner.md"), "z").unwrap();
        symlink(base.join("target.md"), base.join("link.md")).unwrap();
        symlink(base.join("realdir"), base.join("linkdir")).unwrap();
        let mut out = Vec::new();
        collect_files(&base, &base, &[], &mut out);
        let names: Vec<String> = out.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(names.contains(&"real.md".to_string()) && names.contains(&"target.md".to_string()) && names.contains(&"inner.md".to_string()), "real files collected: {names:?}");
        assert!(!names.contains(&"link.md".to_string()), "a file symlink is skipped");
        assert_eq!(names.iter().filter(|n| n.as_str() == "inner.md").count(), 1, "a dir symlink is not descended (inner.md appears once)");
        let _ = fs::remove_dir_all(&base);
    }

    // host-lifecycle#15: the shared worktree walker never follows a symlink, so a
    // symlink cycle in gitignored scratch (a Wine prefix) cannot make it hang. Under
    // the old `is_dir()`-follows-symlink walk this test would not terminate.
    #[cfg(unix)]
    #[test]
    fn walk_files_safe_does_not_follow_a_symlink_cycle() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("hl-walkcycle-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("junk")).unwrap();
        fs::write(base.join("junk/real.obligations"), "x").unwrap();
        symlink(&base, base.join("junk/loop")).unwrap(); // loop -> an ancestor: a cycle
        let files = walk_files_safe(&base, &[]);
        let names: Vec<String> =
            files.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(names.contains(&"real.obligations".to_string()), "real files under the cycle are walked: {names:?}");
        assert!(!files.iter().any(|p| p.to_string_lossy().contains("loop")), "the symlink is never descended: {files:?}");
        let _ = fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // plan/0065 onboarding: the name backstop resolves an explicit value or HOST_NAME,
    // trims whitespace, and returns None only when neither supplies a non-empty name.
    #[test]
    fn resolve_name_from_prefers_explicit_then_env() {
        assert_eq!(resolve_name_from(Some(" acme "), Some("other")).as_deref(), Some("acme"));
        assert_eq!(resolve_name_from(None, Some("fromenv")).as_deref(), Some("fromenv"));
        assert_eq!(resolve_name_from(Some("  "), Some("fromenv")).as_deref(), Some("fromenv"));
        assert_eq!(resolve_name_from(None, None), None);
        assert_eq!(resolve_name_from(Some(""), Some("")), None);
    }

    // The name is held to the host-grammar slug shape, the same the checker accepts.
    #[test]
    fn validate_slug_matches_grammar() {
        assert!(validate_slug("acme").is_ok());
        assert!(validate_slug("my-project").is_ok());
        assert!(validate_slug("app2").is_ok());
        assert!(validate_slug("Acme").is_err(), "uppercase rejected");
        assert!(validate_slug("-x").is_err(), "leading hyphen rejected");
        assert!(validate_slug("x-").is_err(), "trailing hyphen rejected");
        assert!(validate_slug("a--b").is_err(), "double hyphen rejected");
        assert!(validate_slug("").is_err(), "empty rejected");
    }

    // The ruled seed (plan/0065): a purpose line in MEMORY.md, or a default MEMORY when declined.
    #[test]
    fn memory_seed_carries_the_purpose_or_defaults() {
        let seeded = memory_seed_body(Some("  read CAD files for an agent  "));
        assert!(seeded.contains("- Project purpose: read CAD files for an agent\n"));
        assert!(seeded.starts_with("# MEMORY.md"));
        let bare = memory_seed_body(None);
        assert!(!bare.contains("Project purpose"), "no purpose line when declined");
        assert_eq!(memory_seed_body(Some("   ")), bare, "an empty purpose is a decline");
    }

    // The line-based handoff (plan/0065): host-path and next always; remote only when present.
    #[test]
    fn handoff_block_is_line_based() {
        let none = handoff_block("../agentic-acme", None);
        assert_eq!(none, "host-path: ../agentic-acme\nnext: cd ../agentic-acme\n");
        let with = handoff_block("../agentic-acme", Some("git@github.com:me/agentic-acme"));
        assert!(with.contains("remote: git@github.com:me/agentic-acme\n"));
    }

    // init scaffolds a fresh agentic-<name>: the rooms, the stamp at the given revision,
    // the seeded MEMORY purpose, and a README heading. --revision avoids the network.
    #[test]
    fn init_scaffolds_a_fresh_project() {
        let base = std::env::temp_dir().join(format!("hl-init-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        init(&[
            "acme".into(),
            "--at".into(), base.to_string_lossy().into_owned(),
            "--purpose".into(), "reference compiler".into(),
            "--revision".into(), "deadbeef1234".into(),
            "--no-git".into(),
        ]);
        let proj = base.join("agentic-acme");
        assert!(proj.join(".host").is_file(), "stamp written");
        assert!(fs::read_to_string(proj.join(".host")).unwrap().contains("deadbeef1234"), "stamp records the revision");
        for room in ROOMS {
            assert!(proj.join(room).is_dir(), "room {room} scaffolded");
        }
        assert!(fs::read_to_string(proj.join("MEMORY.md")).unwrap().contains("- Project purpose: reference compiler"));
        assert!(fs::read_to_string(proj.join("README.md")).unwrap().contains("# agentic-acme"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn agentic_basename_extracts_the_name() {
        assert_eq!(agentic_basename(Path::new("/x/agentic-acme")).as_deref(), Some("acme"));
        assert_eq!(agentic_basename(Path::new("agentic-my-proj")).as_deref(), Some("my-proj"));
        assert_eq!(agentic_basename(Path::new("/x/notes")), None);
        assert_eq!(agentic_basename(Path::new("/x/agentic-")), None, "empty slug");
        assert_eq!(agentic_basename(Path::new("/x/agentic-Bad")), None, "invalid slug");
    }

    // plan/0065 three-route onboarding: a software repo refuses, an empty agentic-<name>
    // adopts in place (and --force claims a non-empty one), anything else goes elsewhere.
    #[test]
    fn adopt_route_picks_by_folder_shape() {
        let base = std::env::temp_dir().join(format!("hl-route-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let repo = base.join("some-repo");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("Cargo.toml"), "[package]\n").unwrap();
        assert!(matches!(adopt_route(&repo, false), AdoptRoute::Refuse(_)), "software repo refused");

        let named = base.join("agentic-acme");
        fs::create_dir_all(&named).unwrap();
        assert!(matches!(adopt_route(&named, false), AdoptRoute::InPlace(ref n) if n == "acme"), "empty agentic-<name> in place");
        // a freshly git-inited/cloned folder (only .git + a README) still routes in place (F6)
        fs::create_dir_all(named.join(".git")).unwrap();
        fs::write(named.join("README.md"), "# x\n").unwrap();
        assert!(matches!(adopt_route(&named, false), AdoptRoute::InPlace(_)), "a .git+README agentic folder is effectively empty");
        fs::write(named.join("stray.txt"), "x").unwrap();
        assert!(matches!(adopt_route(&named, false), AdoptRoute::Elsewhere), "real content -> not in-place");
        assert!(matches!(adopt_route(&named, true), AdoptRoute::InPlace(_)), "--force claims a non-empty agentic folder");

        let notes = base.join("notes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(notes.join("a.txt"), "x").unwrap();
        assert!(matches!(adopt_route(&notes, false), AdoptRoute::Elsewhere), "arbitrary folder -> elsewhere");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn create_project_builds_a_fresh_host() {
        let base = std::env::temp_dir().join(format!("hl-create-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let target = create_project("acme", &base, Some("reference compiler"), "rev123", false);
        assert_eq!(target, base.join("agentic-acme"));
        assert!(target.join(".host").is_file());
        assert!(target.join("cast").is_dir() && target.join("plan").is_dir() && target.join("call").is_dir());
        assert!(fs::read_to_string(target.join("MEMORY.md")).unwrap().contains("- Project purpose: reference compiler"));
        assert!(fs::read_to_string(target.join("README.md")).unwrap().contains("# agentic-acme"));
        let _ = fs::remove_dir_all(&base);
    }

    // call/0041: the renamed scaffold primitive still writes rooms + the stamp. (The old
    // `adopt <dir> <rev>` two-positional form now hard-errors to protect the source-read-only
    // invariant, so it is exercised through the integration test, not here where exit would abort.)
    #[test]
    fn scaffold_primitive_writes_rooms_and_stamp() {
        let base = std::env::temp_dir().join(format!("hl-scaffold-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        scaffold(&[base.to_string_lossy().into_owned(), "primrev".into()]);
        assert!(base.join(".host").is_file(), "scaffold writes the stamp");
        assert!(fs::read_to_string(base.join(".host")).unwrap().contains("primrev"));
        for room in ROOMS {
            assert!(base.join(room).is_dir(), "room {room} scaffolded");
        }
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0065 oneshot: git_init_commit initialises a repo and commits the scaffold, with a
    // local identity fallback so a machine with no configured git user still commits.
    #[test]
    fn git_init_commit_creates_a_repo_with_a_commit() {
        let base = std::env::temp_dir().join(format!("hl-gitinit-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("README.md"), "# agentic-x\n").unwrap();
        git_init_commit(&base, "scaffold agentic-x").expect("git init + commit");
        assert!(base.join(".git").exists(), ".git created");
        let log = process::Command::new("git")
            .arg("-C").arg(&base).args(["log", "--oneline", "-1"]).output().unwrap();
        assert!(log.status.success() && !log.stdout.is_empty(), "a commit exists");
        assert!(String::from_utf8_lossy(&log.stdout).contains("scaffold agentic-x"));
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0065 shims: the program name maps to the verb; host-lifecycle itself falls through.
    #[test]
    fn shim_verb_maps_program_names() {
        assert_eq!(shim_verb("host-init"), Some("init"));
        assert_eq!(shim_verb("host-adopt"), Some("adopt"));
        assert_eq!(shim_verb("host-lifecycle"), None);
        assert_eq!(shim_verb("git"), None);
    }

    #[test]
    fn stamp_round_trips() {
        let body = stamp_body("abc123", "2026-06-14");
        assert_eq!(parse_revision(&body).as_deref(), Some("abc123"));
        assert!(body.contains(TEMPLATE_URL));
        assert!(body.contains("2026-06-14"));
    }

    #[test]
    fn next_fails_closed_on_numberless_dirs() {
        let base = std::env::temp_dir().join(format!("hl-next-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        // A populated room returns the maximum plus one (unchanged behavior).
        fs::create_dir_all(base.join("plan/0003-c")).unwrap();
        fs::create_dir_all(base.join("plan/0007-g")).unwrap();
        assert_eq!(next_number(&base.join("plan")).ok(), Some(8));
        // A room whose only entry is 0000 returns 1.
        fs::create_dir_all(base.join("call")).unwrap();
        fs::write(base.join("call/0000-x.md"), "# x\n").unwrap();
        assert_eq!(next_number(&base.join("call")).ok(), Some(1));
        // An existing directory with no numbered entries fails closed.
        fs::create_dir_all(base.join("empty")).unwrap();
        assert!(matches!(next_number(&base.join("empty")), Err(NextError::Empty)));
        // The host root (children are rooms, not numbered entries) fails closed.
        assert!(matches!(next_number(&base), Err(NextError::Empty)));
        // A missing path fails closed as a non-directory.
        assert!(matches!(next_number(&base.join("nope")), Err(NextError::NotDir)));
        // The did-you-mean scan names only the known rooms with entries, in room order
        // (a non-room child such as a build directory is never suggested).
        fs::create_dir_all(base.join("book")).unwrap();
        fs::write(base.join("book/404.html"), "x").unwrap();
        assert_eq!(rooms_with_entries(&base), vec!["plan".to_string(), "call".to_string()]);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn entrance_covers_phases_tools_and_stamp() {
        let phases = vec!["classify".to_string(), "release".to_string()];
        let tools = vec!["host-lint".to_string(), "host-lifecycle".to_string()];
        let stamp = entrance_stamp();
        // Names both phases (as backtick tokens), both tools, and a canonical stamp: clean.
        let good = format!("# Entrance\n\nRun `classify` then `release`. Wire host-lint and host-lifecycle.\n\n## The stamp: `.host`\n\n```\n{stamp}```\n\n## Next\n");
        assert!(entrance_problems(&good, &phases, &tools, &Restates::All).is_empty(), "{:?}", entrance_problems(&good, &phases, &tools, &Restates::All));
        // Omitting the `release` phase token flags, even though the bare word recurs in prose.
        let drift = format!("# Entrance\n\nRun `classify`. See the GitHub releases. Wire host-lint and host-lifecycle.\n\n## The stamp: `.host`\n\n```\n{stamp}```\n");
        let probs = entrance_problems(&drift, &phases, &tools, &Restates::All);
        assert_eq!(probs.len(), 1, "{probs:?}");
        assert!(probs[0].contains("release"));
        // Omitting a wired tool flags.
        let notool = format!("# Entrance\n\n`classify` `release`. Wire host-lifecycle.\n\n## The stamp: `.host`\n\n```\n{stamp}```\n");
        assert!(entrance_problems(&notool, &phases, &tools, &Restates::All).iter().any(|p| p.contains("host-lint")));
        // A drifted stamp flags, and regenerate restores the canonical block.
        let badstamp = "# F\n\n`classify` `release` host-lint host-lifecycle.\n\n## The stamp: `.host`\n\n```\nrevision = \"x\"\n```\n";
        assert!(entrance_problems(badstamp, &phases, &tools, &Restates::All).iter().any(|p| p.contains("stamp")));
        let fixed = entrance_regenerate_stamp(badstamp).unwrap();
        assert_eq!(entrance_stamp_block(&fixed).unwrap().trim(), stamp.trim());
        assert!(entrance_problems(&fixed, &phases, &tools, &Restates::All).is_empty(), "{:?}", entrance_problems(&fixed, &phases, &tools, &Restates::All));
    }

    #[test]
    fn parse_revision_handles_missing_and_blank() {
        assert_eq!(parse_revision("template = \"x\"\n"), None);
        assert_eq!(parse_revision("revision = \"\"\n"), None);
        assert_eq!(parse_revision("revision=\"v0.1.0\"\n").as_deref(), Some("v0.1.0"));
    }

    #[test]
    fn parse_manifest_reads_stanzas_modality_and_defaults() {
        let m = "\
[phase \"verify\"]
    order = 7
    command = host-lifecycle verify
    skill = verify
    skippable = false

[phase \"release\"]
    order = 8
    modality = conditional-on-Where, recurring-per-component
    command = host-lifecycle release
    skill = release
    evidence = attestation + tag
    requires = verify
";
        let p = parse_manifest(m);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "verify");
        assert_eq!(p[0].order, 7);
        assert!(!p[0].skippable, "verify is the protected core (skippable = false)");
        assert!(p[1].skippable, "skippable defaults to true when absent");
        assert!(p[1].recurring());
        assert!(!p[0].recurring());
        assert_eq!(p[1].conditional_on(), Some("Where"));
        assert_eq!(p[0].conditional_on(), None);
        assert_eq!(p[1].requires, vec!["verify".to_string()]);
        assert_eq!(p[0].requires, Vec::<String>::new());
    }

    #[test]
    fn receipts_parse_and_latest_wins() {
        let text = "\
[receipt \"verify\"]
    disposition = done
    evidence = gate:x

[receipt \"release\" \"host-lint\"]
    disposition = skip
    reason = call/0031

[receipt \"verify\"]
    disposition = done
    evidence = gate:y
";
        let rs = parse_receipts(text);
        assert_eq!(rs.len(), 3);
        assert_eq!(latest_receipt(&rs, "verify", None).unwrap().evidence.as_deref(), Some("gate:y"));
        assert_eq!(latest_receipt(&rs, "release", Some("host-lint")).unwrap().disposition, "skip");
        assert!(latest_receipt(&rs, "release", None).is_none());
    }

    #[test]
    fn record_validation_enforces_invariants() {
        assert!(validate_receipt_record("verify", "n-a", None, None, true).is_err(), "n-a is tool-only");
        assert!(validate_receipt_record("build", "done", None, None, true).is_err(), "done needs evidence");
        assert!(validate_receipt_record("build", "done", Some("tag:v1"), None, true).is_ok());
        assert!(validate_receipt_record("verify", "skip", None, None, false).is_err(), "skip needs a reason");
        assert!(validate_receipt_record("verify", "skip", None, Some("call/1"), false).is_err(), "protected core");
        assert!(validate_receipt_record("embed", "skip", None, Some("call/1"), true).is_ok());
        assert!(validate_receipt_record("x", "maybe", None, None, true).is_err(), "unknown disposition");
    }

    #[test]
    fn gate_flags_missing_na_and_protected() {
        let phases = parse_manifest("\
[phase \"verify\"]
    order = 1
    command = c
    skill = verify
    skippable = false
    recheck = true
[phase \"release\"]
    order = 2
    modality = conditional-on-Where, recurring-per-component
    command = c
    skill = release
    recheck = true
");
        // Where room + one component, no receipts: both phases HAZARD; the release
        // line carries a literal --component remedy (the de-risk fold-back).
        let g = receipt_gate(&phases, &[], true, &["host-lint".into()]);
        assert_eq!(g.len(), 2);
        assert!(g.iter().all(|l| !l.ok));
        let rel = g.iter().find(|l| l.label.contains("release")).unwrap();
        assert!(rel.remedy.as_deref().unwrap().contains("--component host-lint"));

        // No Where room: release is tool-computed n-a; verify still needs a receipt.
        let g2 = receipt_gate(&phases, &[], false, &[]);
        let rel2 = g2.iter().find(|l| l.label.contains("release")).unwrap();
        assert!(rel2.ok && rel2.note.contains("n-a"));

        // done verify with a declared recheck → ok + a recheck to run.
        let done = parse_receipts("[receipt \"verify\"]\n    disposition = done\n    evidence = x\n");
        let v = receipt_gate(&phases, &done, false, &[]).into_iter().find(|l| l.label.contains("verify")).unwrap();
        assert!(v.ok && v.recheck.is_some());

        // skip of the protected core → HAZARD.
        let skip = parse_receipts("[receipt \"verify\"]\n    disposition = skip\n    reason = call/1\n");
        let vs = receipt_gate(&phases, &skip, false, &[]).into_iter().find(|l| l.label.contains("verify")).unwrap();
        assert!(!vs.ok && vs.note.contains("protected"));
    }

    #[test]
    fn next_version_maps_class_pre_and_post_1_0() {
        // pre-1.0: breaking AND feature bump the minor (this project's convention —
        // every host-lifecycle 0.N.0 was a feature); only a fix is a patch.
        assert_eq!(next_version("0.4.2", &ChangeClass::Breaking).unwrap(), "0.5.0");
        assert_eq!(next_version("0.4.2", &ChangeClass::Feature).unwrap(), "0.5.0");
        assert_eq!(next_version("0.4.2", &ChangeClass::Fix).unwrap(), "0.4.3");
        // a leading `v` is tolerated (tool versions read from a `v*` tag).
        assert_eq!(next_version("v0.18.1", &ChangeClass::Feature).unwrap(), "0.19.0");
        // post-1.0: textbook semver.
        assert_eq!(next_version("1.4.2", &ChangeClass::Breaking).unwrap(), "2.0.0");
        assert_eq!(next_version("1.4.2", &ChangeClass::Feature).unwrap(), "1.5.0");
        assert_eq!(next_version("1.4.2", &ChangeClass::Fix).unwrap(), "1.4.3");
        // malformed input is an error, never a silent guess.
        assert!(next_version("0.4", &ChangeClass::Fix).is_err());
        assert!(next_version("0.4.x", &ChangeClass::Fix).is_err());
    }

    #[test]
    fn change_class_is_concrete_not_a_semver_level() {
        // the agent answers the concrete change, never `major|minor|patch`.
        assert!(matches!(parse_change_class("removes-flag"), Some(ChangeClass::Breaking)));
        assert!(matches!(parse_change_class("adds-flag"), Some(ChangeClass::Feature)));
        assert!(matches!(parse_change_class("neither"), Some(ChangeClass::Fix)));
        assert!(parse_change_class("minor").is_none(), "a semver level is not a change class (the Fen fumble)");
    }

    #[test]
    fn cargo_version_round_trips_and_skips_other_tables() {
        let toml = "[package]\nname = \"host-lint\"\nversion = \"0.4.2\"\n\n[dependencies]\nserde = { version = \"1.0\" }\n";
        assert_eq!(cargo_version(toml).as_deref(), Some("0.4.2"));
        let bumped = set_cargo_version(toml, "0.5.0").unwrap();
        assert_eq!(cargo_version(&bumped).as_deref(), Some("0.5.0"));
        // the dependency version in [dependencies] is untouched.
        assert!(bumped.contains("serde = { version = \"1.0\" }"));
        // no [package] version → None (a tool with no crate manifest).
        assert!(set_cargo_version("[workspace]\nmembers = []\n", "1.0.0").is_none());
    }

    #[test]
    fn cargo_version_falls_back_to_workspace_package() {
        // A virtual workspace root (no [package]) keeps the version in [workspace.package]; a
        // workspace component releases through the same read-and-bump path as a single crate.
        let toml = "[workspace]\nmembers = [\"crates/cli\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2021\"\n";
        assert_eq!(cargo_version(toml).as_deref(), Some("0.1.0"));
        let bumped = set_cargo_version(toml, "0.2.0").unwrap();
        assert_eq!(cargo_version(&bumped).as_deref(), Some("0.2.0"));
        assert!(bumped.contains("[workspace.package]"));
        // a root [package] still wins over [workspace.package] when both are present.
        let both = "[package]\nname = \"x\"\nversion = \"1.2.3\"\n\n[workspace.package]\nversion = \"9.9.9\"\n";
        assert_eq!(cargo_version(both).as_deref(), Some("1.2.3"));
    }

    #[test]
    fn workspace_members_and_inheritance_parse() {
        let root = "[workspace]\nresolver = \"2\"\nmembers = [\"crates/core\", \"crates/cli\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n";
        assert_eq!(workspace_members(root), vec!["crates/core", "crates/cli"]);
        // a member that inherits the workspace version moves with the bump …
        assert!(inherits_workspace_version("[package]\nname = \"c\"\nversion.workspace = true\n"));
        assert!(inherits_workspace_version("[package]\nname = \"c\"\nversion = { workspace = true }\n"));
        // … one with a literal version does not.
        assert!(!inherits_workspace_version("[package]\nname = \"c\"\nversion = \"2.0.0\"\n"));
    }

    #[test]
    fn skip_citation_must_be_bare_call_id() {
        assert!(valid_skip_citation("call/0031"));
        assert!(!valid_skip_citation("reproducible-build/0031"), "the Fen fumble: phase name, not a call id");
        assert!(!valid_skip_citation("call/"));
        assert!(!valid_skip_citation("call/12a"));
        assert!(!valid_skip_citation("0031"));
    }

    #[test]
    fn classify_by_governance() {
        assert_eq!(classify_case(true, true), "c"); // stamp wins
        assert_eq!(classify_case(true, false), "c");
        assert_eq!(classify_case(false, true), "b");
        assert_eq!(classify_case(false, false), "a");
    }

    #[test]
    fn refuse_adopting_software_in_place() {
        let base = std::env::temp_dir().join(format!("hl-refuse-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        // empty / greenfield → proceed
        assert_eq!(adopt_in_place_refusal(&base), None);

        // a root build manifest → refuse, naming the manifest
        fs::write(base.join("Cargo.toml"), "[package]\n").unwrap();
        assert_eq!(adopt_in_place_refusal(&base), Some("Cargo.toml"));

        // a stamp means it is already a host (case c) → proceed
        fs::write(base.join(STAMP), "revision = \"x\"\n").unwrap();
        assert_eq!(adopt_in_place_refusal(&base), None);
        fs::remove_file(base.join(STAMP)).unwrap();

        // already managing software via .host-software → proceed
        fs::write(base.join(SOFTWARE), "[software \"x\"]\n").unwrap();
        assert_eq!(adopt_in_place_refusal(&base), None);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(31), (1970, 2, 1));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(365), (1971, 1, 1)); // 1970 not a leap year
        assert_eq!(civil_from_days(20_617), (2026, 6, 13));
        assert_eq!(civil_from_days(20_618), (2026, 6, 14));
    }

    #[test]
    fn decision_field_reads_madr_headers() {
        let t = "# T\n\n- Status: accepted\n- Scope: host-lint\n";
        assert_eq!(decision_field(t, "Status").as_deref(), Some("accepted"));
        assert_eq!(decision_field(t, "Scope").as_deref(), Some("host-lint"));
        assert_eq!(decision_field(t, "Date"), None);
    }

    #[test]
    fn scope_gate_passes_and_fails() {
        // accepted + software scope: ok
        assert!(decision_scope_problem("- Status: accepted\n- Scope: host-lint\n").is_none());
        // accepted + methodology: fails (ouroboros)
        assert!(decision_scope_problem("- Status: accepted\n- Scope: methodology\n").is_some());
        // accepted, no scope: fails
        assert!(decision_scope_problem("- Status: accepted\n").is_some());
        // superseded: not in force, passes regardless of scope
        assert!(decision_scope_problem("- Status: superseded by the spine\n").is_none());
    }

    // plan/0036: an accepted decision whose Scope names host-template authored a rule
    // now resident in the spine, so it must be superseded there (the call/0017 class).
    #[test]
    fn scope_gate_flags_host_template_scope() {
        // accepted + a Scope that names host-template (alongside a tool): fails
        assert!(decision_scope_problem("- Status: accepted\n- Scope: host-lifecycle, host-template (the spine)\n").is_some());
        // accepted + a tool-only scope that happens to start with host-: still ok
        assert!(decision_scope_problem("- Status: accepted\n- Scope: host-lint, host-prove\n").is_none());
        // superseded: the status short-circuit wins regardless of host-template scope
        assert!(decision_scope_problem("- Status: superseded by the spine\n- Scope: host-template\n").is_none());
    }
}

#[cfg(test)]
mod software_tests {
    use super::*;

    // plan/0048: a non-rung token has no re-deriver to probe, so the check returns None
    // without spawning any subprocess. (The runnable/not-runnable branches for a real rung
    // are environment-dependent — host-prove + the verifier on PATH — and are integration-
    // tested by `software --check`, not here, so the suite stays portable.)
    #[test]
    fn rung_rederiver_problem_ignores_non_rung_tokens() {
        assert!(rung_rederiver_problem("test:").is_none());
        assert!(rung_rederiver_problem("structural").is_none());
        assert!(rung_rederiver_problem("").is_none());
    }

    #[test]
    fn parses_multi_component_recipe() {
        let text = "\
# a comment
[software \"alpha\"]
\turl       = https://example.test/alpha.git
\tpin       = aaaa1111
\tworktrees = alpha.oauth alpha.fix

[software \"beta\"]
\turl  = https://example.test/beta.git
\tpin  = bbbb2222
\tworktrees =
";
        let s = parse_software(text);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "alpha");
        assert_eq!(s[0].url, "https://example.test/alpha.git");
        assert_eq!(s[0].pin, "aaaa1111");
        assert_eq!(s[0].worktrees, vec!["alpha.oauth", "alpha.fix"]);
        assert_eq!(s[1].name, "beta");
        assert!(s[1].worktrees.is_empty());
    }

    #[test]
    fn unknown_keys_ignored() {
        let s = parse_software("[software \"x\"]\nurl = u\npin = p\nbogus = ignored\n");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].url, "u");
        assert_eq!(s[0].pin, "p");
        assert!(s[0].lines.is_empty());
    }

    // Reproducible-build provenance fields parse (issue #10).
    #[test]
    fn parses_build_provenance() {
        let text = "\
[software \"ik\"]
\turl          = https://x.test/ik.git
\tpin          = abc123
\tbuild        = cmake -B build && cmake --build build
\ttoolchain    = gcc-13
\tdeploy       = ik
\tartifact     = build/bin/srv deadbeefcafe
\trepro-exempt = call/0009
";
        let s = parse_software(text);
        assert_eq!(s[0].build.as_deref(), Some("cmake -B build && cmake --build build"));
        assert_eq!(s[0].toolchain.as_deref(), Some("gcc-13"));
        assert_eq!(s[0].deploy.as_deref(), Some("ik"));
        assert_eq!(s[0].artifact, Some(("build/bin/srv".to_string(), "deadbeefcafe".to_string())));
        assert_eq!(s[0].repro_exempt.as_deref(), Some("call/0009"));
    }

    // The fast attestation/exemption pass: deploy-line recorded, artifact hash matches,
    // exemption cites a real decision (issue #10).
    #[test]
    fn provenance_attestation_and_exemption() {
        let base = std::env::temp_dir().join(format!("hl-prov-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("software/ik/main/build/bin")).unwrap();
        fs::create_dir_all(base.join("call")).unwrap();
        fs::write(base.join("software/ik/main/build/bin/srv"), "BINARY").unwrap();
        fs::write(base.join("call/0009-exempt.md"), "# x\n- Status: accepted\n- Scope: ik\n").unwrap();
        let sha = sha256_file(&base.join("software/ik/main/build/bin/srv")).unwrap();

        // A `toolchain` pin is present (host#14): the artifact-hash checks below test
        // the local-build *note* path, not the no-toolchain HAZARD path (tested after).
        let mk = |deploy: &str, art_sha: &str, exempt: Option<&str>| Software {
            name: "ik".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: Some("gcc-13".into()),
            deploy: Some(deploy.into()),
            artifact: Some(("build/bin/srv".into(), art_sha.into())),
            repro_exempt: exempt.map(String::from), hooks: None, deps_bundle: None, builds: vec![],
        };
        // recorded deploy line + matching artifact hash + valid exemption → clean
        assert_eq!(provenance_problems(&base, &mk("ik", &sha, Some("call/0009"))), 0);
        // a non-matching artifact hash is a local-build *note*, not a failure: the
        // recorded hash is the pinned container's output, and --verify-build is the
        // reproducibility proof (the worktree-at-pin gate lives in software_check).
        assert_eq!(provenance_problems(&base, &mk("ik", "0000", None)), 0);
        // unrecorded deploy line → 1; exemption citing a missing decision → 1 (so 2)
        assert_eq!(provenance_problems(&base, &mk("ghost", &sha, Some("call/9999"))), 2);

        // host#14: an artifact with no `toolchain` pin is a HAZARD (not reproducibly
        // verifiable) — unless an exemption excuses it.
        let no_tc = |exempt: Option<&str>| Software {
            name: "ik".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None,
            deploy: Some("ik".into()),
            artifact: Some(("build/bin/srv".into(), sha.clone())),
            repro_exempt: exempt.map(String::from), hooks: None, deps_bundle: None, builds: vec![],
        };
        assert_eq!(provenance_problems(&base, &no_tc(None)), 1);
        assert_eq!(provenance_problems(&base, &no_tc(Some("call/0009"))), 0);

        let _ = fs::remove_dir_all(&base);
    }

    // `[build "<name>" "<platform>"]` subsections parse into per-platform builds
    // sharing the component pin (issue #1); flat fields stay empty.
    #[test]
    fn parses_platform_builds() {
        let text = "\
[software \"ik\"]
\turl = https://x.test/ik.git
\tpin = abc123
[build \"ik\" \"linux-cuda\"]
\ttoolchain   = nvidia/cuda:12
\tbuild        = cmake --preset cuda
\tartifact     = build/srv aaaa
\tdeploy       = ik
\tattest-host  = linux
[build \"ik\" \"windows-msvc-cuda\"]
\tbuild        = cmake --preset msvc
\tartifact     = build/srv.exe bbbb
\tattest-host  = windows
\trepro-exempt = call/0009
";
        let s = parse_software(text);
        assert_eq!(s.len(), 1);
        assert!(s[0].build.is_none(), "flat build stays empty when [build] sections drive it");
        assert_eq!(s[0].builds.len(), 2);
        assert_eq!(s[0].builds[0].platform, "linux-cuda");
        assert_eq!(s[0].builds[0].attest_host.as_deref(), Some("linux"));
        assert_eq!(s[0].builds[0].artifact, Some(("build/srv".into(), "aaaa".into())));
        assert_eq!(s[0].builds[1].platform, "windows-msvc-cuda");
        assert_eq!(s[0].builds[1].repro_exempt.as_deref(), Some("call/0009"));
        // builds_view yields the two explicit builds, not a flat default.
        let v = s[0].builds_view();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].platform, Some("linux-cuda"));
    }

    // A foreign-platform build is skipped, not failed: only the build whose
    // `attest-host` matches the current OS is attested (issue #1).
    #[test]
    fn foreign_platform_build_is_skipped_not_failed() {
        let base = std::env::temp_dir().join(format!("hl-plat-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("ik")).unwrap();
        // A build pinned to a host that is never this test runner: its artifact is
        // absent and its hash is wrong, yet it must not count as a failure.
        let other = if std::env::consts::OS == "linux" { "windows" } else { "linux" };
        let s = Software {
            name: "ik".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None, deps_bundle: None,
            builds: vec![PlatformBuild {
                platform: "foreign".into(),
                build: None, toolchain: None, deploy: None,
                artifact: Some(("build/srv".into(), "0000".into())),
                repro_exempt: None,
                attest_host: Some(other.into()),
            }],
        };
        assert_eq!(provenance_problems(&base, &s), 0, "foreign-host build is skipped, not failed");
        let _ = fs::remove_dir_all(&base);
    }

    // === plan/0074: the shared call site, and what each concern writes there ===

    // One materialize call produces BOTH artifacts, and they share no field name:
    // the receipt's keys are event-level, the fingerprint's are state-level. The
    // verifying concerns write neither.
    #[test]
    fn materialize_envhash_write_records_state_facts() {
        let base = std::env::temp_dir().join(format!("hl-callsite-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git_ok(cwd, args), "git {args:?}");
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("readme.txt"), "seed").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "seed"]);
        let pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        let recipe = vec![Software {
            name: "demo".into(), url: src.to_string_lossy().to_string(), pin,
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];

        software_materialize(&host, &recipe, false);
        let receipt = fs::read_to_string(host.join(LIFECYCLE_RECEIPTS)).expect("the event was recorded");
        let fingerprint = fs::read_to_string(host.join(envhash::ENVHASH)).expect("the state was recorded");

        // Field names, one artifact against the other: the two sets are disjoint, so
        // neither file can drift into restating the other's facts.
        let keys = |text: &str| -> Vec<String> {
            text.lines()
                .filter_map(|l| l.trim().split_once(" = ").map(|(k, _)| k.to_string()))
                .collect()
        };
        for k in keys(&receipt) {
            assert!(!keys(&fingerprint).contains(&k), "`{k}` appears in both the receipt and the fingerprint");
        }
        assert!(keys(&receipt).contains(&"disposition".to_string()), "the receipt records the event");
        assert!(fingerprint.contains("[envhash \"repo_path\"]"), "the fingerprint records the machine");

        // The verifying concerns read: the gate changes neither file, and reading the
        // bootstrap sequence against the tree changes nothing at all.
        let before = (receipt.clone(), fingerprint.clone());
        let _ = setup::verify_setup(&host, &recipe);
        for kind in bootstrap::SEQUENCE {
            let _ = bootstrap::read_step(&host, &recipe, kind);
        }
        assert_eq!(
            (fs::read_to_string(host.join(LIFECYCLE_RECEIPTS)).unwrap(), fs::read_to_string(host.join(envhash::ENVHASH)).unwrap()),
            before,
            "the completeness gate and the step planner write nothing"
        );
        let _ = fs::remove_dir_all(&base);
    }

    // === plan/0074: the materialize receipt (#18), event facts only ===

    fn materialize_fixture(name: &str, toolchain: Option<&str>) -> Software {
        Software {
            name: name.into(),
            url: "https://example.invalid/x.git".into(),
            pin: "241a8703f2b1c4d5e6f708192a3b4c5d6e7f8091".into(),
            branch: "main".into(),
            worktrees: vec![],
            lines: vec![],
            build: None,
            toolchain: toolchain.map(|t| t.into()),
            deploy: None,
            artifact: None,
            repro_exempt: None,
            hooks: None,
            deps_bundle: None,
            builds: vec![],
        }
    }

    // The receipt's evidence names the component and carries the pin by reference: it is
    // the event that happened, and the pin is context, never an adherence claim.
    #[test]
    fn materialize_receipt_names_its_components() {
        let base = std::env::temp_dir().join(format!("hl-mr-names-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let s = materialize_fixture("host-lint", None);
        append_materialize_receipt(&base, &s, 2);
        let rs = read_all_receipts(&base);
        let r = rs.iter().find(|r| r.phase == "materialize").expect("materialize receipt written");
        assert_eq!(r.component.as_deref(), Some("host-lint"));
        assert!(r.evidence.as_deref().unwrap().contains("241a8703f2b1"), "the pin rides as a reference");
        assert_eq!(r.disposition, "done");
        let _ = fs::remove_dir_all(&base);
    }

    // One realized component appends exactly one stanza, and a second realized run appends
    // another beside it: the file is append-only, so nothing is rewritten.
    #[test]
    fn realized_run_appends_one_receipt() {
        let base = std::env::temp_dir().join(format!("hl-mr-append-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let s = materialize_fixture("host-prove", None);
        append_materialize_receipt(&base, &s, 1);
        let after_first = fs::read_to_string(base.join(LIFECYCLE_RECEIPTS)).unwrap();
        append_materialize_receipt(&base, &s, 3);
        let text = fs::read_to_string(base.join(LIFECYCLE_RECEIPTS)).unwrap();
        assert_eq!(text.matches("[receipt \"materialize\" \"host-prove\"]").count(), 2);
        assert!(text.starts_with(&after_first), "the earlier stanza survives the append verbatim");
        let _ = fs::remove_dir_all(&base);
    }

    // The write records the toolchain image REFERENCE and never its digest: the digest is
    // an envhash dimension, and the two artifacts share no fact (plan/0074's field table).
    #[test]
    fn materialize_receipt_write_records_event_facts() {
        let tc = "docker.io/clux/muslrust:1.95.0-stable@sha256:15a72a4abf1c593b0bea63a4a8f20e95";
        assert_eq!(image_reference(tc), "docker.io/clux/muslrust:1.95.0-stable");
        let s = materialize_fixture("host-lifecycle", Some(tc));
        let e = materialize_evidence(&s, 2);
        assert!(e.contains("docker.io/clux/muslrust:1.95.0-stable"), "the image reference is recorded");
        assert!(!e.contains("sha256:"), "the image digest never crosses into the receipt");
    }

    // The state facts the envhash owns never appear in a receipt: no digest, no hook binary
    // hash, no absolute repo path, no submodule init state.
    #[test]
    fn receipt_records_only_event_facts() {
        let base = std::env::temp_dir().join(format!("hl-mr-facts-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let s = materialize_fixture("host-grammar", Some("ghcr.io/x/toolchain:1@sha256:deadbeef"));
        append_materialize_receipt(&base, &s, 1);
        let text = fs::read_to_string(base.join(LIFECYCLE_RECEIPTS)).unwrap();
        for state_fact in ["sha256:", "submodule", "/home/", "hook"] {
            assert!(!text.contains(state_fact), "receipt carries the state fact `{state_fact}`");
        }
        for event_fact in ["disposition = done", "recorded = ", "tool = host-lifecycle@"] {
            assert!(text.contains(event_fact), "receipt is missing the event fact `{event_fact}`");
        }
        let _ = fs::remove_dir_all(&base);
    }

    // A materialized component carrying a spec must have its CI lane: a `.allium`
    // without `allium check`+`analyse`, or a `.tla` without TLC, is a HAZARD.
    #[test]
    fn spec_lane_gate_requires_a_lane_when_a_spec_is_present() {
        let base = std::env::temp_dir().join(format!("hl-lane-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("comp").join("main");
        fs::create_dir_all(wt.join(".github/workflows")).unwrap();
        fs::write(wt.join("thing.allium"), "-- allium: 3\n").unwrap();
        let mk = || Software {
            name: "comp".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        };
        // .allium present, no workflow + no manifest → 2 HAZARDs
        assert_eq!(spec_lane_problems(&base, &mk()), 2);
        // a workflow running check + analyse clears one; still missing the manifest
        fs::write(wt.join(".github/workflows/allium.yml"), "run: allium check x\nrun: allium analyse x\n").unwrap();
        assert_eq!(spec_lane_problems(&base, &mk()), 1);
        // the obligations manifest clears the rest
        fs::write(wt.join("thing.obligations"), "x => structural\n").unwrap();
        assert_eq!(spec_lane_problems(&base, &mk()), 0);
        // add a .tla with no TLC lane → HAZARD again
        fs::write(wt.join("Spec.tla"), "---- MODULE Spec ----\n").unwrap();
        assert_eq!(spec_lane_problems(&base, &mk()), 1);
        // a TLC lane clears it
        fs::write(wt.join(".github/workflows/specula.yml"), "run: java -cp tla2tools.jar tlc2.TLC Spec.tla\n").unwrap();
        assert_eq!(spec_lane_problems(&base, &mk()), 0);
        // an un-materialized component is skipped, not failed
        assert_eq!(spec_lane_problems(&base, &Software { name: "absent".into(), ..mk() }), 0);
        let _ = fs::remove_dir_all(&base);
    }

    // Deep-verification tiers are opt-in and inert: a HAZARD fires only when a
    // `.obligations` manifest declares the tier and its CI lane is absent. With no
    // declaration the worktree is inert (0), even though it materializes fine.
    #[test]
    fn tier_lanes_are_opt_in_and_inert() {
        let base = std::env::temp_dir().join(format!("hl-tier-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("comp").join("main");
        fs::create_dir_all(wt.join(".github/workflows")).unwrap();
        let mk = || Software {
            name: "comp".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        };
        // inert: no spec, no tier declaration → no HAZARD
        assert_eq!(spec_lane_problems(&base, &mk()), 0);
        // declare two tiers with no lanes wired → one HAZARD each, nothing else
        fs::write(wt.join("p.obligations"), "a => kani:verify_thing\nb => apalache:Inv\n").unwrap();
        assert_eq!(spec_lane_problems(&base, &mk()), 2);
        // wiring the Kani lane clears only the Kani HAZARD
        fs::write(wt.join(".github/workflows/kani.yml"), "run: cargo kani\n").unwrap();
        assert_eq!(spec_lane_problems(&base, &mk()), 1);
        // wiring the Apalache lane clears the rest
        fs::write(wt.join(".github/workflows/apalache.yml"), "run: apalache-mc check Spec.tla\n").unwrap();
        assert_eq!(spec_lane_problems(&base, &mk()), 0);
        let _ = fs::remove_dir_all(&base);
    }

    // issue #13: the tier gate parses dispositions (parse_rung), so a `kani:` in a comment or a
    // non-rung disposition does not activate a tier and demand a CI lane the engine never treats
    // as a rung.
    #[test]
    fn tier_declaration_parses_dispositions_not_substrings() {
        let base = std::env::temp_dir().join(format!("hl-tier-decl-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("comp").join("main");
        fs::create_dir_all(wt.join(".github/workflows")).unwrap();
        let mk = || Software {
            name: "comp".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        };
        // a comment mentioning `kani:`, a real apalache rung, and a non-rung `test:` disposition.
        fs::write(wt.join("p.obligations"), "# kani: is discussed here, not declared\nInvSafe => apalache:Inv spec=S.tla\nFoo => test:foo\n").unwrap();
        assert_eq!(declared_rung_tokens(&wt), vec!["apalache:"], "only the parsed rung is declared");
        // so only the apalache lane is missing → exactly one HAZARD, never a phantom kani demand.
        assert_eq!(spec_lane_problems(&base, &mk()), 1);
        let _ = fs::remove_dir_all(&base);
    }

    // Obligation discharge: every `allium plan` id must be dispositioned; stale
    // dispositions and absent `test:` references are flagged.
    #[test]
    fn obligation_gaps_require_full_disposition() {
        // allium plan is pretty-printed: each obligation's `"id"` on its own line.
        let plan = "{\n  \"obligations\": [\n    {\n      \"id\": \"rule-success.DetectPhaseSynonym\",\n      \"category\": \"rule_success\"\n    },\n    {\n      \"id\": \"transition-edge.Check.scanning.blocked\"\n    },\n    {\n      \"id\": \"entity-fields.Line\"\n    }\n  ]\n}";
        let ids = extract_obligation_ids(plan);
        assert_eq!(ids.len(), 3);

        // a manifest covering two of three, with one stale entry
        let manifest = parse_obligation_manifest(
            "# map\n\
             rule-success.DetectPhaseSynonym => test:phase_synonym_flags\n\
             entity-fields.Line => structural\n\
             gone.obligation => waived: removed\n",
        );
        // transition-edge... is MISSING; gone.obligation is STALE → 2 problems
        let p = obligation_gaps(&ids, &manifest, None, None);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p.iter().any(|x| x.contains("MISSING") && x.contains("transition-edge")));
        assert!(p.iter().any(|x| x.contains("STALE") && x.contains("gone.obligation")));

        // full, non-stale manifest → clean
        let full = parse_obligation_manifest(
            "rule-success.DetectPhaseSynonym => test:phase_synonym_flags\n\
             transition-edge.Check.scanning.blocked => test:flag_blocks\n\
             entity-fields.Line => structural\n",
        );
        assert!(obligation_gaps(&ids, &full, None, None).is_empty());
        // with test sources: the named test must exist
        let p2 = obligation_gaps(&ids, &full, Some("fn flag_blocks() {}"), None);
        assert!(p2.iter().any(|x| x.contains("ABSENT") && x.contains("phase_synonym_flags")));
        assert_eq!(obligation_gaps(&ids, &full, Some("fn phase_synonym_flags(){} fn flag_blocks(){}"), None).len(), 0);
    }

    // Deep-verification tiers: a `kani:`/`apalache:`/`tlaps:` disposition is valid,
    // and when prove sources are supplied the named harness/invariant/theorem must
    // occur in them (the proof analog of the `test:` existence check).
    #[test]
    fn obligation_gaps_validates_proof_tiers() {
        let ids = vec!["b.Numeral".to_string(), "c.Verdict".to_string()];
        let m = parse_obligation_manifest(
            "b.Numeral => kani:verify_is_dotted_code\n\
             c.Verdict => tlaps:Safety\n",
        );
        // no prove sources: the tier dispositions are accepted (like `structural`)
        assert!(obligation_gaps(&ids, &m, None, None).is_empty());
        // prove sources missing the proof names → ABSENT for each
        let p = obligation_gaps(&ids, &m, None, Some("fn other() {}\nTHEOREM Unrelated == TRUE"));
        assert!(p.iter().any(|x| x.contains("ABSENT") && x.contains("kani:verify_is_dotted_code")), "{p:?}");
        assert!(p.iter().any(|x| x.contains("ABSENT") && x.contains("tlaps:Safety")), "{p:?}");
        // prove sources containing both names → clean
        let src = "fn verify_is_dotted_code() {}\nTHEOREM Safety == TRUE";
        assert!(obligation_gaps(&ids, &m, None, Some(src)).is_empty());
    }

    #[test]
    fn obligation_gaps_strips_rung_suffixes_from_the_proof_name() {
        // A rung with `bound=`/`spec=`/`inputs=` must match on the NAME alone, not
        // the whole disposition (the dogfood bug: `inputs=` made every rung ABSENT).
        let ids = vec!["a.X".to_string()];
        let m = parse_obligation_manifest("a.X => kani:verify_h bound=unwind=20 inputs=src/lib.rs\n");
        assert!(obligation_gaps(&ids, &m, None, Some("fn verify_h() {}")).is_empty(), "name should match despite the suffix");
        // The real ABSENT (name truly missing) still fires.
        assert!(!obligation_gaps(&ids, &m, None, Some("fn other() {}")).is_empty());
    }

    // plan/0052 (no-hollow-green): the strengthened `test:` discharge link. The
    // confirmed hole was a disposition pointed at a test that drives nothing; the
    // `exercises=` token must occur in the named test's body, the name must resolve to
    // exactly one definition, and an unlinked or substring-only `test:` is flagged.
    #[test]
    fn discharge_warnings_catch_hollow_unlinked_and_ambiguous() {
        let src = "fn a_drives() { let _ = software_check(&p); }\n\
                   fn b_helper_only() { assert!(escapes_root(\"x\")); }\n";
        // HOLLOW: b declares it drives software_check but its body references only a helper.
        let ids = vec!["r.A".to_string(), "r.B".to_string()];
        let m = parse_obligation_manifest(
            "r.A => test:a_drives exercises=software_check\n\
             r.B => test:b_helper_only exercises=software_check\n",
        );
        let w = discharge_warnings(&ids, &m, Some(src));
        assert!(w.iter().any(|x| x.contains("HOLLOW") && x.contains("r.B")), "{w:?}");
        assert!(!w.iter().any(|x| x.contains("r.A")), "a linked test must not warn: {w:?}");

        // UNLINKED: a `test:` with no `exercises=` is the staged requirement.
        let m2 = parse_obligation_manifest("r.A => test:a_drives\n");
        let w2 = discharge_warnings(&["r.A".to_string()], &m2, Some(src));
        assert!(w2.iter().any(|x| x.contains("UNLINKED") && x.contains("r.A")), "{w2:?}");

        // AMBIGUOUS: two definitions of the same name.
        let dup = "fn a_drives() {}\nfn a_drives() {}\n";
        let w3 = discharge_warnings(&["r.A".to_string()], &m2, Some(dup));
        assert!(w3.iter().any(|x| x.contains("AMBIGUOUS")), "{w3:?}");

        // substring-only: a name that is not a real definition, only a substring, is
        // flagged UNLINKED rather than silently passing (the plan/0051 hole).
        let sub = "fn a_drives_extra() {}\n";
        let w4 = discharge_warnings(&["r.A".to_string()], &m2, Some(sub));
        assert!(w4.iter().any(|x| x.contains("UNLINKED") && x.contains("substring only")), "{w4:?}");

        // IGNORED: a discharging test that is #[ignore]'d never runs, so it discharges nothing.
        let ign_src = "#[test]\n#[ignore]\nfn a_drives() { let _ = software_check(&p); }\n";
        let w_ign = discharge_warnings(&["r.A".to_string()], &m2, Some(ign_src));
        assert!(w_ign.iter().any(|x| x.contains("IGNORED") && x.contains("r.A")), "{w_ign:?}");

        // no test sources supplied: the `test:` checks are skipped (offline/cheap path).
        assert!(discharge_warnings(&ids, &m, None).is_empty());

        // RELABEL: a behavioural obligation dispositioned `structural` is the relabel dodge.
        let relabel = parse_obligation_manifest("rule-success.X => structural\n");
        let w5 = discharge_warnings(&["rule-success.X".to_string()], &relabel, None);
        assert!(w5.iter().any(|x| x.contains("RELABEL")), "{w5:?}");
        // a genuinely structural obligation dispositioned `structural` is fine.
        let ok = parse_obligation_manifest("entity-fields.Y => structural\n");
        assert!(discharge_warnings(&["entity-fields.Y".to_string()], &ok, None).is_empty());

        // UNWAIVED: an empty waiver reason is flagged; a real reason (StartCheck) is not.
        let empty = parse_obligation_manifest("rule-success.Z => waived:\n");
        let w6 = discharge_warnings(&["rule-success.Z".to_string()], &empty, None);
        assert!(w6.iter().any(|x| x.contains("UNWAIVED")), "{w6:?}");
        let reasoned = parse_obligation_manifest("rule-success.Z => waived: lifecycle entry, no observable assertion\n");
        assert!(discharge_warnings(&["rule-success.Z".to_string()], &reasoned, None).is_empty());
    }

    // `--rederive` (call/0018): discharge is the verifier PASSING at the declared bound,
    // re-run via host-prove — not name-presence. The verdict-interpretation is pure.
    #[test]
    fn rung_disposition_parses() {
        let r = parse_rung("kani:verify_x bound=unwind=20").unwrap();
        assert_eq!((r.tool.as_str(), r.name.as_str(), r.bound.as_deref()), ("kani", "verify_x", Some("unwind=20")));
        let a = parse_rung("apalache:Inv spec=Scan.tla bound=length=12").unwrap();
        assert_eq!((a.tool.as_str(), a.name.as_str(), a.spec.as_deref(), a.bound.as_deref()), ("apalache", "Inv", Some("Scan.tla"), Some("length=12")));
        assert!(parse_rung("test:some_test").is_none());
        assert!(parse_rung("structural").is_none());
    }

    #[test]
    fn rung_parses_inputs_for_staleness() {
        let r = parse_rung("kani:verify_x bound=unwind=20 inputs=src/lib.rs,src/main.rs").unwrap();
        assert_eq!(r.inputs, vec!["src/lib.rs".to_string(), "src/main.rs".to_string()]);
        // No `inputs=` → empty (the staleness signal is opt-in per rung).
        assert!(parse_rung("kani:verify_x bound=unwind=20").unwrap().inputs.is_empty());
    }

    #[test]
    fn digest_ledger_round_trips() {
        let dir = std::env::temp_dir().join(format!("hl-ledger-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let led = dir.join("m.obligations.digests");
        fs::write(&led, "# header\n\nrule-success.A\tdeadbeef\nrule-success.B\tcafef00d,1234\n").unwrap();
        let got = read_digest_ledger(&led);
        assert_eq!(got, vec![
            ("rule-success.A".to_string(), "deadbeef".to_string()),
            ("rule-success.B".to_string(), "cafef00d,1234".to_string()),
        ]);
        // A missing ledger is empty (feature off), not an error.
        assert!(read_digest_ledger(&dir.join("absent.digests")).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn staleness_detects_an_input_change() {
        let dir = std::env::temp_dir().join(format!("hl-stale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("foo.rs"), "fn a() {}\n").unwrap();
        let disp = vec![("rule-success.X".to_string(), "kani:h inputs=foo.rs".to_string())];
        let led = digest_ledger_path(&dir.join("m.obligations"));
        // Record the fingerprint, then an unchanged check is clean.
        assert_eq!(record_digests(&disp, &dir, &led).unwrap(), 1);
        assert!(staleness_problems(&disp, &dir, &led).is_empty(), "fresh record must not be stale");
        // Change the proven input → STALE.
        fs::write(dir.join("foo.rs"), "fn b() {}\n").unwrap();
        let probs = staleness_problems(&disp, &dir, &led);
        assert_eq!(probs.len(), 1, "{probs:?}");
        assert!(probs[0].contains("STALE"), "{probs:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_staleness_problems_catch_a_staled_digest() {
        // The release gate's allium-free arm (plan/0069): parse an obligations manifest,
        // resolve its sibling digest ledger, and report a STALE rung. This is the born-red
        // class caught at release time without needing allium plan or the test sources.
        let dir = std::env::temp_dir().join(format!("hl-mstale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("foo.rs"), "fn a() {}\n").unwrap();
        let manifest = dir.join("m.obligations");
        fs::write(&manifest, "# c\nrule-success.X => kani:h inputs=foo.rs\n").unwrap();
        // Fresh record: clean.
        let disp = parse_obligation_manifest(&fs::read_to_string(&manifest).unwrap());
        let ledger = digest_ledger_path(&manifest);
        assert_eq!(record_digests(&disp, &dir, &ledger).unwrap(), 1);
        assert!(manifest_staleness_problems(&manifest).is_empty(), "fresh record must not be stale");
        // Change the proven input: STALE through the manifest path.
        fs::write(dir.join("foo.rs"), "fn b() {}\n").unwrap();
        let probs = manifest_staleness_problems(&manifest);
        assert_eq!(probs.len(), 1, "{probs:?}");
        assert!(probs[0].contains("STALE"), "{probs:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn seed_lexicon_writes_a_warn_free_scaffold() {
        let dir = std::env::temp_dir().join(format!("hl-seed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        seed_lexicon(&dir, false);
        let body = fs::read_to_string(dir.join("LEXICON")).unwrap();
        assert!(body.contains("host-lint: strict"));
        assert!(body.contains("host-lint: jira-key PROJ"));
        // No line is the LIVE strict directive (a `# host-lint: strict` with no
        // trailing text, which host-lint's loader would honour). The scaffold's
        // directive lines carry trailing guidance, so seeding never blocks a repo.
        let live = body.lines().any(|l| l.trim().trim_start_matches('#').trim() == "host-lint: strict");
        assert!(!live, "scaffold must not enable strict");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verdict_discharges_only_on_pass_at_bound() {
        let kani = parse_rung("kani:verify_x bound=unwind=20").unwrap();
        // PASS at an adequate bound → discharged
        assert!(verdict_discharges(&kani, "SUCCESSFUL verify_x [bound=unwind=20]").is_ok());
        assert!(verdict_discharges(&kani, "SUCCESSFUL verify_x [bound=unwind=40]").is_ok());
        // a real negative / error verdict → NOT discharged (the #8 bug: name-presence would pass)
        assert!(verdict_discharges(&kani, "FAILED verify_x (replay: …)").is_err());
        assert!(verdict_discharges(&kani, "ERROR verify_x: cargo kani could not run").is_err());
        // PASS but UNDER the declared bound → not discharged (#9)
        assert!(verdict_discharges(&kani, "SUCCESSFUL verify_x [bound=unwind=10]").is_err());
        // PASS but bound unspecified though one was declared → not discharged (#9)
        assert!(verdict_discharges(&kani, "SUCCESSFUL verify_x [bound=unspecified]").is_err());
        // no declared bound → PASS word alone suffices
        let nob = parse_rung("kani:verify_x").unwrap();
        assert!(verdict_discharges(&nob, "SUCCESSFUL verify_x [bound=unspecified]").is_ok());
        // apalache PROVEN / tlaps ALL-PROVED [unbounded]
        let ap = parse_rung("apalache:Inv bound=length=12").unwrap();
        assert!(verdict_discharges(&ap, "PROVEN Inv [bound=length=12]").is_ok());
        assert!(verdict_discharges(&ap, "VIOLATED Inv (counterexample: x.tla)").is_err());
        let tl = parse_rung("tlaps:Safety").unwrap();
        assert!(verdict_discharges(&tl, "ALL-PROVED Safety (3 obligations) [unbounded]").is_ok());
        assert!(verdict_discharges(&tl, "FAILED Safety: 1/3 (first: 9:1:9:5)").is_err());
    }

    // #12: a spec under plan/*/spec/ is a HAZARD (specs co-locate with software).
    #[test]
    fn plan_spec_under_spec_dir_hazards() {
        let base = std::env::temp_dir().join(format!("hl-planspec-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("plan/0099-x/spec")).unwrap();
        fs::create_dir_all(base.join("plan/0098-y")).unwrap();
        fs::write(base.join("plan/0099-x/spec/Foo.tla"), "---- MODULE Foo ----").unwrap();
        fs::write(base.join("plan/0099-x/README.md"), "ok").unwrap(); // not a spec, not under spec/
        fs::write(base.join("plan/0098-y/README.md"), "ok").unwrap(); // milestone with no spec/ dir
        assert_eq!(plan_spec_problems(&base), 1);
        let _ = fs::remove_dir_all(&base);
    }

    // A `hooks` key parses into the optional hook-script path.
    #[test]
    fn parses_hooks_field() {
        let s = parse_software("[software \"hl\"]\nurl = u\npin = p\nhooks = pre-commit\n");
        assert_eq!(s[0].hooks.as_deref(), Some("pre-commit"));
        // absent by default
        let t = parse_software("[software \"hl\"]\nurl = u\npin = p\n");
        assert_eq!(t[0].hooks, None);
    }

    // install-hooks copies the dispatch script (as pre-commit + commit-msg) and the
    // deploy binary into the repo's hooks dir when the worktree is at its pin; a
    // worktree off its pin, or a missing binary, blocks it.
    #[test]
    fn install_hooks_copies_script_and_binary_at_pin() {
        let base = std::env::temp_dir().join(format!("hl-hooks-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("hl").join("main");
        fs::create_dir_all(wt.join("target/release")).unwrap();
        let git = |dir: &Path, args: &[&str]| process::Command::new("git")
            .arg("-C").arg(dir).args(args).status().map(|s| s.success()).unwrap_or(false);
        assert!(git(&base, &["init", "-q"]), "git init host");
        // The worktree is its own checkout; commit so it has a HEAD = the pin.
        assert!(git(&wt, &["init", "-q"]));
        assert!(git(&wt, &["config", "user.email", "t@t"]) && git(&wt, &["config", "user.name", "t"]));
        fs::write(wt.join("pre-commit"), "#!/bin/bash\nexit 0\n").unwrap();
        fs::write(wt.join("target/release/host-lint"), "BINARY").unwrap();
        assert!(git(&wt, &["add", "-A"]) && git(&wt, &["commit", "-qm", "x"]));
        let pin = git_out(&wt, &["rev-parse", "HEAD"]).unwrap();

        let mk = |pin: &str, art: &str| Software {
            name: "hl".into(), url: "u".into(), pin: pin.into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: Some("host-lint".into()),
            artifact: Some(("target/release/host-lint".into(), art.into())),
            repro_exempt: None, hooks: Some("pre-commit".into()), deps_bundle: None, builds: vec![],
        };
        // worktree at pin + binary present → installs all three files, into the host
        // repo AND the materialized worktree: a worktree is a real commit surface, so
        // it carries its own local gate (plan/0074, Bug A).
        assert_eq!(install_hooks(&base, &[mk(&pin, "deadbeef")]), (2, 0));
        let hooks = base.join(".git/hooks");
        assert!(hooks.join("pre-commit").is_file());
        assert!(hooks.join("commit-msg").is_file());
        assert!(hooks.join("host-lint").is_file());
        let wt_hooks = wt.join(".git/hooks");
        for f in ["pre-commit", "commit-msg", "host-lint"] {
            assert!(wt_hooks.join(f).is_file(), "the worktree is gated too: {f} missing");
        }
        // worktree off its pin → blocked
        assert_eq!(install_hooks(&base, &[mk("0000000000000000000000000000000000000000", "x")]), (0, 1));

        // plan/0074: installing hooks moved the binary's hash, so the op refreshes the
        // fingerprint — and appends no receipt, because a state change is not an event.
        software_install_hooks(&base, &[mk(&pin, "deadbeef")]);
        let fingerprint = fs::read_to_string(base.join(envhash::ENVHASH)).expect("install-hooks records the fingerprint");
        assert!(fingerprint.contains("[envhash \"hook_binary\"]"));
        assert!(!base.join(LIFECYCLE_RECEIPTS).exists(), "install-hooks appends no provenance");

        let _ = fs::remove_dir_all(&base);
    }

    // The explicit `worktree = <dir> <branch> <pin>` form parses into a fully-pinned
    // parallel line, alongside the bare dir-list form (issue #6).
    #[test]
    fn parses_explicit_worktree_lines() {
        let text = "\
[software \"ik\"]
\turl       = https://example.test/ik.git
\tpin       = b217881
\tworktree  = perf/256k-single-context a0506f2
";
        let s = parse_software(text);
        assert_eq!(s.len(), 1);
        assert!(s[0].worktrees.is_empty());
        assert_eq!(s[0].lines.len(), 1);
        assert_eq!(s[0].lines[0].branch, "perf/256k-single-context");
        assert_eq!(s[0].lines[0].pin, "a0506f2");
    }

    // A parallel line materializes on its own branch at its own pin — not the
    // canonical pin the bare dir-list form would have used (issue #6).
    #[cfg(unix)]
    #[test]
    fn explicit_worktree_lands_on_its_own_branch_and_pin() {
        let base = std::env::temp_dir().join(format!("hl-wt-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git_ok(cwd, args), "git {args:?}");
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("a.txt"), "one").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "one"]);
        let canon = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        // a second commit on a feature branch — the parallel line's pin
        g(&src, &["checkout", "-q", "-b", "feature"]);
        fs::write(src.join("b.txt"), "two").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "two"]);
        let line_pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        g(&src, &["checkout", "-q", "main"]);

        let recipe = vec![Software {
            name: "demo".to_string(),
            url: src.to_string_lossy().to_string(),
            pin: canon.clone(),
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: vec![Worktree {
                branch: "feature".to_string(),
                pin: line_pin.clone(),
                store: None,
                host: None,
            }],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        software_materialize(&host, &recipe, false);

        let line = host.join("software").join("demo").join("feature");
        assert!(line.is_dir(), "parallel worktree created");
        // at its OWN pin, not the canonical one
        assert_eq!(git_out(&line, &["rev-parse", "HEAD"]).unwrap(), line_pin);
        assert_ne!(line_pin, canon, "fixture sanity: the two pins differ");
        assert_eq!(git_out(&line, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap(), "feature");
        software_check(&host, &recipe); // passes on a matching branch+pin

        let _ = fs::remove_dir_all(&base);
    }

    // `--item <name>[@<branch>]` narrows the recipe to one component, and `@<branch>`
    // narrows further to that worktree at the matching pin (plan/0029).
    #[test]
    fn filter_item_narrows_to_component_and_branch() {
        let recipe = parse_software(
            "[software \"a\"]\n url=u\n pin=p1\n worktree = feature q9deadbeef\n\
             [software \"b\"]\n url=u\n pin=p2\n",
        );
        let only_a = filter_item(recipe.clone(), "a");
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].name, "a");
        assert_eq!(only_a[0].lines.len(), 1);
        // a line's branch → that worktree at the LINE's pin, nothing else
        let a_feat = filter_item(recipe.clone(), "a@feature");
        assert_eq!(a_feat.len(), 1);
        assert_eq!(a_feat[0].branch, "feature");
        assert_eq!(a_feat[0].pin, "q9deadbeef");
        assert!(a_feat[0].lines.is_empty() && a_feat[0].worktrees.is_empty());
        // the canonical branch → the component pin
        let a_main = filter_item(recipe, "a@main");
        assert_eq!(a_main[0].branch, "main");
        assert_eq!(a_main[0].pin, "p1");
    }

    #[test]
    fn host_root_escape_is_detected() {
        // The wrong-tree footgun: an absolute or `..`-climbing worktree path.
        assert!(escapes_root("/mnt/d/dev/ik_llama.cpp"));
        assert!(escapes_root("../outside"));
        assert!(escapes_root("a/../../escape"));
        // In-tree paths are fine, including a descent that stays under root.
        assert!(!escapes_root("ik_llama.cpp.windows"));
        assert!(!escapes_root("nested/handle"));
        assert!(!escapes_root("a/b/../c"));
    }

    #[test]
    fn recipe_problems_flags_duplicate_names_and_escaping_paths() {
        // A clean single-component recipe has no recipe-level problem.
        let clean = parse_software("[software \"x\"]\n url=u\n pin=p\n");
        assert!(recipe_problems(&clean).is_empty());
        // Issue #16: a duplicate `[software "<name>"]` stanza — the second is silently
        // ignored by materialize/release, so the parser must reject it.
        let mut dup = parse_software("[software \"x\"]\n url=u\n pin=p\n");
        dup.extend(parse_software("[software \"x\"]\n url=v\n pin=q\n"));
        assert!(recipe_problems(&dup).iter().any(|p| p.contains("duplicate")));
        // Issue #21: an absolute artifact path escapes the throwaway worktree.
        let mut abs = parse_software("[software \"x\"]\n url=u\n pin=p\n");
        abs[0].artifact = Some(("/etc/passwd".into(), "deadbeef".into()));
        assert!(recipe_problems(&abs).iter().any(|p| p.contains("artifact") && p.contains("escapes")));
        // Issue #21: a `..`-climbing hooks path escapes the worktree.
        let mut hooks = parse_software("[software \"x\"]\n url=u\n pin=p\n");
        hooks[0].hooks = Some("../../evil".into());
        assert!(recipe_problems(&hooks).iter().any(|p| p.contains("hooks") && p.contains("escapes")));
    }

    #[test]
    fn worktree_escapes_catches_name_branch_and_lines() {
        // Issue #2: materialize fails closed when any recorded branch/name escapes.
        let clean = parse_software("[software \"x\"]\n url=u\n pin=p\n");
        assert!(worktree_escapes(&clean[0]).is_none());
        let mut bad_name = parse_software("[software \"x\"]\n url=u\n pin=p\n");
        bad_name[0].name = "../x".into();
        assert_eq!(worktree_escapes(&bad_name[0]), Some("../x"));
        let mut bad_branch = parse_software("[software \"x\"]\n url=u\n pin=p\n");
        bad_branch[0].branch = "a/../../b".into();
        assert!(worktree_escapes(&bad_branch[0]).is_some());
        // A `worktree =` line branch is covered too.
        let mut bad_line = parse_software("[software \"x\"]\n url=u\n pin=p\n worktree = ../up abc\n");
        assert!(worktree_escapes(&bad_line[0]).is_some());
        bad_line[0].lines.clear();
        assert!(worktree_escapes(&bad_line[0]).is_none());
    }

    #[test]
    fn worktree_line_parses_store_and_host() {
        let s = parse_software(
            "[software \"x\"]\n  url = u\n  pin = p\n  \
             worktree = windows/msvc abc123 store=/mnt/d/dev/ik host=linux\n",
        );
        let w = &s[0].lines[0];
        assert_eq!(w.branch, "windows/msvc");
        assert_eq!(w.pin, "abc123");
        assert_eq!(w.store.as_deref(), Some("/mnt/d/dev/ik"));
        assert_eq!(w.host.as_deref(), Some("linux"));
        // back-compat: a bare 2-token line carries no store/host
        let b = parse_software("[software \"x\"]\n url=u\n pin=p\n worktree = br pin\n");
        assert!(b[0].lines[0].store.is_none() && b[0].lines[0].host.is_none());
    }

    // An external-store line materializes the git worktree at the store and an
    // in-tree symlink handle to it; --check resolves the handle to the store (#2).
    #[cfg(unix)]
    #[test]
    fn external_store_line_materializes_an_in_tree_handle() {
        let base = std::env::temp_dir().join(format!("hl-store-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        let store = base.join("external").join("ik"); // a path NOT under host/
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        fs::create_dir_all(base.join("external")).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git_ok(cwd, args), "git {args:?}");
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("a.txt"), "one").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "one"]);
        let canon = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        g(&src, &["checkout", "-q", "-b", "win"]);
        fs::write(src.join("b.txt"), "two").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "two"]);
        let line_pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        g(&src, &["checkout", "-q", "main"]);

        let recipe = vec![Software {
            name: "demo".to_string(),
            url: src.to_string_lossy().to_string(),
            pin: canon,
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: vec![Worktree {
                branch: "win".to_string(),
                pin: line_pin.clone(),
                store: Some(store.to_string_lossy().to_string()),
                host: None,
            }],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        software_materialize(&host, &recipe, false);

        let handle = host.join("software").join("demo").join("win");
        // the in-tree handle exists and is a symlink to the external store
        assert!(fs::symlink_metadata(&handle).unwrap().file_type().is_symlink(), "handle is a symlink");
        assert_eq!(fs::canonicalize(&handle).unwrap(), fs::canonicalize(&store).unwrap());
        // the git worktree physically lives at the store, at the line pin
        assert!(store.join(".git").exists(), "worktree at the external store");
        assert_eq!(git_out(&store, &["rev-parse", "HEAD"]).unwrap(), line_pin);
        software_check(&host, &recipe); // passes: handle resolves to store, pin+branch match

        let _ = fs::remove_dir_all(&base);
    }

    // host-lifecycle#14: `--partial` clones the bare store with --filter=blob:none, and
    // the pin still round-trips — the worktree fetches the pinned tree's blobs from the
    // promisor remote at checkout (a `file://` url so git honours the filter).
    #[cfg(unix)]
    #[test]
    fn materialize_partial_clone_still_pins() {
        let base = std::env::temp_dir().join(format!("hl-software-partial-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git_ok(cwd, args), "git {args:?} failed");
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("readme.txt"), "seed").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "seed"]);
        let pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        let recipe = vec![Software {
            name: "demo".to_string(),
            url: format!("file://{}", src.display()),
            pin: pin.clone(),
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: Vec::new(),
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        software_materialize(&host, &recipe, true);
        let canon = host.join("software").join("demo").join("main");
        assert!(canon.is_dir(), "canonical worktree created from a partial clone");
        assert_eq!(git_out(&canon, &["rev-parse", "HEAD"]).unwrap(), pin, "the pin round-trips through a partial clone");
        assert_eq!(fs::read_to_string(canon.join("readme.txt")).unwrap(), "seed", "the pinned blob was fetched on checkout");
        let _ = fs::remove_dir_all(&base);
    }

    // Materialise from a local source repo, then check the pin round-trips.
    #[cfg(unix)]
    #[test]
    fn materialize_then_check_roundtrip() {
        let base = std::env::temp_dir().join(format!("hl-software-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        let g = |cwd: &Path, args: &[&str]| {
            assert!(git_ok(cwd, args), "git {args:?} failed in {}", cwd.display());
        };
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("readme.txt"), "seed").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "seed"]);
        let pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();

        let recipe = vec![Software {
            name: "demo".to_string(),
            url: src.to_string_lossy().to_string(),
            pin: pin.clone(),
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: Vec::new(),
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        software_materialize(&host, &recipe, false);

        // The bare store is `.bare`, with a `.git` gitdir-link file beside it (call/0039).
        assert!(host.join("software").join("demo").join(".bare").is_dir(), "bare store created at .bare");
        assert_eq!(
            fs::read_to_string(host.join("software").join("demo").join(".git")).unwrap(),
            "gitdir: ./.bare\n",
            "the .git gitdir-link points at .bare"
        );
        let canon = host.join("software").join("demo").join("main");
        assert!(canon.is_dir(), "canonical worktree created");
        assert_eq!(git_out(&canon, &["rev-parse", "HEAD"]).unwrap(), pin);
        // A clean check is clean (VerdictClean): a matching pin yields no hazard.
        assert_eq!(software_check(&host, &recipe), 0, "matching pin is clean");

        // plan/0074 (#18): the run that realized the worktrees appended exactly one
        // materialize receipt for the component, and it records the event, not the tree.
        let receipts = |h: &Path| {
            fs::read_to_string(h.join(LIFECYCLE_RECEIPTS))
                .unwrap_or_default()
                .matches("[receipt \"materialize\" \"demo\"]")
                .count()
        };
        assert_eq!(receipts(&host), 1, "one realized run, one receipt");
        // The same call site refreshed the fingerprint (plan/0074's two orthogonal
        // writers), and the two files share no fact by name.
        let fingerprint = fs::read_to_string(host.join(envhash::ENVHASH)).expect("materialize records the fingerprint");
        assert!(fingerprint.contains("[envhash \"worktree_paths\"]"), "the fingerprint records the tree");
        for event_fact in ["disposition", "evidence", "recorded ="] {
            assert!(!fingerprint.contains(event_fact), "the fingerprint carries the event fact `{event_fact}`");
        }
        assert!(
            !fs::read_to_string(host.join(LIFECYCLE_RECEIPTS)).unwrap().contains(&host.to_string_lossy().to_string()),
            "the receipt carries no absolute repo path (an envhash dimension)"
        );

        // issue #8 review: the gitdir-link gate FIRES when `.git` is missing, and a re-materialize
        // self-heals the link (call/0039) — the negative path the pass-only assertions missed.
        let gitlink = host.join("software").join("demo").join(".git");
        fs::remove_file(&gitlink).unwrap();
        assert!(software_check(&host, &recipe) > 0, "a missing .git gitdir-link hazards the check");
        software_materialize(&host, &recipe, false);
        assert_eq!(fs::read_to_string(&gitlink).unwrap(), "gitdir: ./.bare\n", "re-materialize self-heals the .git link");
        assert_eq!(software_check(&host, &recipe), 0, "self-healed link is clean again");
        // A run that realized nothing is not an event: the idempotent re-run appends no
        // second receipt (plan/0074).
        assert_eq!(receipts(&host), 1, "a no-op re-materialize records nothing");

        // Re-materialize after the worktree is removed: `worktree prune` clears the
        // stale admin entry, so the canonical is re-created rather than hard-failing
        // with "missing but already registered" (plan/0029).
        fs::remove_dir_all(&canon).unwrap();
        assert!(!canon.is_dir());
        software_materialize(&host, &recipe, false);
        assert!(canon.is_dir(), "canonical re-created after removal + prune");
        assert_eq!(git_out(&canon, &["rev-parse", "HEAD"]).unwrap(), pin);
        // That re-materialize realized a worktree again, so it is a second event beside
        // the first: the provenance file accretes, it never rewrites.
        assert_eq!(receipts(&host), 2, "the second realizing run appends a second receipt");

        let _ = fs::remove_dir_all(&base);
    }

    // issue #8 HIGH (review): --materialize migrates an existing plan/0029-layout component (a bare
    // repo named `.git`) to call/0039's `.bare` store + `.git` gitdir-link, repairing the worktree,
    // rather than exiting EISDIR and leaving a stray `.bare`. The recorded pin reproduces, so the
    // migration is a rename plus a `worktree repair`, no network or teardown.
    #[test]
    fn materialize_migrates_old_git_dir_layout() {
        let base = std::env::temp_dir().join(format!("hl-miglayout-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git_ok(cwd, args), "git {args:?} failed in {}", cwd.display());
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("readme.txt"), "seed").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "seed"]);
        let pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();

        // Build the OLD layout by hand: a bare repo named `.git` inside the component dir, with the
        // canonical worktree beside it (what plan/0029 materialize produced).
        let comp = host.join("software").join("demo");
        fs::create_dir_all(&comp).unwrap();
        let old_git = comp.join(".git");
        g(&host, &["clone", "-q", "--bare", &src.to_string_lossy(), &old_git.to_string_lossy()]);
        g(&old_git, &["worktree", "add", "-q", "-B", "main", &comp.join("main").to_string_lossy(), &pin]);
        assert!(old_git.is_dir(), "old .git bare store is a directory");

        let recipe = vec![Software {
            name: "demo".to_string(),
            url: src.to_string_lossy().to_string(),
            pin: pin.clone(),
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: Vec::new(),
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        // Before migration the new-layout check hazards (no `.bare`).
        assert!(software_check(&host, &recipe) > 0, "old layout hazards the new check");
        // Migrate.
        software_materialize(&host, &recipe, false);
        assert!(comp.join(".bare").is_dir(), "migrated to .bare");
        assert!(comp.join(".git").is_file(), ".git is now a gitdir-link file");
        assert_eq!(fs::read_to_string(comp.join(".git")).unwrap(), "gitdir: ./.bare\n");
        // The worktree survives the migration at its pin (`worktree repair` fixed its gitdir link).
        assert_eq!(git_out(&comp.join("main"), &["rev-parse", "HEAD"]).unwrap(), pin, "worktree repaired to its pin");
        assert_eq!(software_check(&host, &recipe), 0, "migrated component is clean");
        // Idempotent: a second materialize neither re-migrates nor fails.
        software_materialize(&host, &recipe, false);
        assert_eq!(software_check(&host, &recipe), 0, "second materialize stays clean");

        let _ = fs::remove_dir_all(&base);
    }

    // issue #9 / call/0038: the pure reader that extracts what the template's prose CI pins
    // host-lifecycle at, in three states.
    #[test]
    fn template_hostlc_pin_reads_the_pinned_sha() {
        let yaml = "      - name: install\n        run: cargo install --git https://github.com/connollydavid/host-lifecycle --rev deadbeefcafe0 --root \"$HOME/.local\"\n";
        assert!(matches!(template_hostlc_pin(yaml), TemplatePin::Rev(ref s) if s == "deadbeefcafe0"));
        // the `--rev=<sha>` form is also read
        assert!(matches!(template_hostlc_pin("run: cargo install --git x/host-lifecycle --rev=abc1234def\n"), TemplatePin::Rev(ref s) if s == "abc1234def"));
        // no host-lifecycle install -> NoInstall (legitimately inert)
        assert!(matches!(template_hostlc_pin("run: cargo build\n"), TemplatePin::NoInstall));
        assert!(matches!(template_hostlc_pin("# uses host-lifecycle prose\n"), TemplatePin::NoInstall));
        // installs host-lifecycle but names no --rev -> fail closed
        assert!(matches!(template_hostlc_pin("run: cargo install --git x/host-lifecycle --tag v1\n"), TemplatePin::InstallNoRev));
    }

    // issue #9 review: SHA equality is a case-insensitive prefix match (git abbreviates), floored
    // at 7 chars so a stub does not match everything.
    #[test]
    fn sha_eq_matches_abbreviations_not_stubs() {
        assert!(sha_eq("deadbeefcafe", "deadbeefcafe0123")); // abbreviation of the same commit
        assert!(sha_eq("DEADBEEF0", "deadbeef0000")); // case-insensitive
        assert!(!sha_eq("deadbeef0", "beefdead0000")); // genuinely different
        assert!(!sha_eq("abc", "abc1234")); // too short (< 7) never matches
    }

    // issue #9 review: the release emits the template-pin-bump steps for host-lifecycle only, each
    // a concrete command (including the submodule-pointer bump).
    #[test]
    fn template_pin_bump_lines_cover_every_carried_tool() {
        let carried = vec!["host-lifecycle".to_string(), "host-lint".to_string()];
        // host-lifecycle is pinned two ways: the prose-CI --rev install AND the tools/host-lifecycle submodule.
        let hl = template_pin_bump_lines("host-lifecycle", "0.36.0", &carried);
        assert!(hl.iter().any(|l| l.contains("prose.yml") && l.contains("0.36.0")), "host-lifecycle names the prose.yml --rev bump");
        assert!(hl.iter().any(|l| l.contains("tools/host-lifecycle")), "host-lifecycle names the tools/host-lifecycle submodule bump");
        assert!(hl.iter().any(|l| l.contains("git add host-template")), "names the host submodule-pointer bump");
        // host-lint is pinned one way: the tools/host-lint submodule.
        let lint = template_pin_bump_lines("host-lint", "1.0.0", &carried);
        assert!(lint.iter().any(|l| l.contains("tools/host-lint")), "host-lint names the tools/host-lint submodule bump");
        assert!(!lint.iter().any(|l| l.contains("prose.yml")), "host-lint has no prose.yml pin");
        // A tool in the derived carried set the template did not historically pin (host-foo) gets its bump — the Fen bar.
        let foo = template_pin_bump_lines("host-foo", "0.1.0", &["host-foo".to_string()]);
        assert!(foo.iter().any(|l| l.contains("tools/host-foo")), "a new carried tool gets its tools/<name> bump");
        // A component not in the carried set gets no steps.
        assert!(template_pin_bump_lines("host-reference", "0.1.6", &carried).is_empty(), "no steps for a component the template does not pin");
    }

    #[test]
    fn carried_template_tools_returns_the_intersection() {
        // Fen bar (plan/0069): a producer repo that develops a new family tool (host-foo) AND
        // carries it in host-template/tools/ is carried, with no code change. External
        // referenced tools (allium) and non-distributed components (host-reference) are excluded.
        let base = std::env::temp_dir().join(format!("hl-carried-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let tdir = base.join("host-template");
        fs::create_dir_all(&tdir).unwrap();
        fs::write(tdir.join("UPGRADING.md"), "# ledger\n").unwrap();
        fs::write(tdir.join(".gitmodules"),
            "[submodule \"allium\"]\n\tpath = tools/allium\n\n[submodule \"host-lint\"]\n\tpath = tools/host-lint\n\n[submodule \"host-lifecycle\"]\n\tpath = tools/host-lifecycle\n\n[submodule \"host-foo\"]\n\tpath = tools/host-foo\n").unwrap();
        let mk = |name: &str| Software {
            name: name.into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        };
        let recipe = vec![mk("host-lint"), mk("host-lifecycle"), mk("host-foo"), mk("host-reference")];
        let mut carried = carried_template_tools(&base, &recipe);
        carried.sort();
        assert_eq!(carried, vec!["host-foo".to_string(), "host-lifecycle".to_string(), "host-lint".to_string()],
            "carried = .host-software ∩ tools/ (excludes allium the external tool and host-reference the non-distributed component)");
        let _ = fs::remove_dir_all(&base);
    }

    // issue #9 / call/0038: the template-pin gate HAZARDs when the template's prose CI pins a
    // host-lifecycle commit other than the recorded pin (or pins none while installing it), is
    // clean when they match (prefix-tolerant), and is inert where this repo does not develop
    // host-lifecycle or carries no template.
    #[test]
    fn template_pin_gate_hazards_a_stale_prose_pin() {
        let base = std::env::temp_dir().join(format!("hl-tmplpin-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wf = base.join("host-template").join(".github").join("workflows");
        fs::create_dir_all(&wf).unwrap();
        fs::write(base.join("host-template").join("UPGRADING.md"), "# ledger\n").unwrap();
        let prose = |rev: &str| format!("run: cargo install --git https://github.com/connollydavid/host-lifecycle --rev {rev} --root x\n");
        let mk = |name: &str, pin: &str| Software {
            name: name.into(), url: "u".into(), pin: pin.into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        };
        let hostlc = vec![mk("host-lifecycle", "abc123def4567")];
        // stale: prose pins a different commit than the recipe -> HAZARD
        fs::write(wf.join("prose.yml"), prose("0000staleaaaaa")).unwrap();
        assert_eq!(template_pin_problems(&base, &hostlc), 1, "a drifted template pin hazards");
        // equal by prefix: prose pins an abbreviation of the recorded commit -> clean
        fs::write(wf.join("prose.yml"), prose("abc123def")).unwrap();
        assert_eq!(template_pin_problems(&base, &hostlc), 0, "a matching (abbreviated) template pin is clean");
        // installs host-lifecycle but names no --rev -> fail closed
        fs::write(wf.join("prose.yml"), "run: cargo install --git x/host-lifecycle --tag v1\n").unwrap();
        assert_eq!(template_pin_problems(&base, &hostlc), 1, "an unpinned host-lifecycle install hazards");
        // inert when this repo does not develop host-lifecycle, even with a stale prose pin
        fs::write(wf.join("prose.yml"), prose("0000staleaaaaa")).unwrap();
        assert_eq!(template_pin_problems(&base, &[mk("host-prove", "abc123def4567")]), 0, "inert without a host-lifecycle recipe entry");
        // inert when there is no template at all
        let _ = fs::remove_dir_all(base.join("host-template"));
        assert_eq!(template_pin_problems(&base, &hostlc), 0, "inert without a template submodule");
        let _ = fs::remove_dir_all(&base);
    }

    // issue #9 / call/0038 (gap fix): the gate also checks host-template's submodule gitlinks
    // (`tools/host-lifecycle`, `tools/host-lint`), not just prose.yml — these were the most stale
    // pins in practice (a host-lifecycle submodule at v0.15.1). A drifted gitlink HAZARDs; a
    // matching one is clean; an absent submodule is inert.
    #[test]
    fn template_submodule_pin_gate_hazards_drift() {
        let base = std::env::temp_dir().join(format!("hl-tmplsub-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let tdir = base.join("host-template");
        fs::create_dir_all(&tdir).unwrap();
        fs::write(tdir.join("UPGRADING.md"), "# ledger\n").unwrap(); // find_template_dir locates it
        let g = |args: &[&str]| assert!(git_ok(&tdir, args), "git {args:?} failed in {}", tdir.display());
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        // The carried set is derived from host-template/.gitmodules ∩ .host-software (plan/0069),
        // so the fixture's .gitmodules must list the tools whose gitlinks drift below.
        fs::write(tdir.join(".gitmodules"),
            "[submodule \"host-lifecycle\"]\n\tpath = tools/host-lifecycle\n[submodule \"host-lint\"]\n\tpath = tools/host-lint\n").unwrap();
        g(&["add", ".gitmodules"]);
        g(&["commit", "-qm", "gitmodules"]);
        let recorded = "486add7227d1cf86f16ea4462a08d4efe0fca159"; // the recorded .host-software pin
        let stale = "2a24deb0e5bcb3b3c09f50c39d7cfb84c445eafa"; // an older commit
        let mk = |name: &str, pin: &str| Software {
            name: name.into(), url: "u".into(), pin: pin.into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        };
        // a stale tools/host-lifecycle gitlink -> HAZARD (no prose.yml here, so this isolates it)
        g(&["update-index", "--add", "--cacheinfo", &format!("160000,{stale},tools/host-lifecycle")]);
        g(&["commit", "-qm", "gitlink stale"]);
        assert_eq!(template_pin_problems(&base, &[mk("host-lifecycle", recorded)]), 1, "a drifted submodule gitlink hazards");
        // bump the gitlink to the recorded pin -> clean
        g(&["update-index", "--add", "--cacheinfo", &format!("160000,{recorded},tools/host-lifecycle")]);
        g(&["commit", "-qm", "gitlink current"]);
        assert_eq!(template_pin_problems(&base, &[mk("host-lifecycle", recorded)]), 0, "a matching submodule gitlink is clean");
        // add a stale tools/host-lint gitlink -> that one HAZARDs while host-lifecycle stays clean
        g(&["update-index", "--add", "--cacheinfo", &format!("160000,{stale},tools/host-lint")]);
        g(&["commit", "-qm", "host-lint gitlink stale"]);
        assert_eq!(
            template_pin_problems(&base, &[mk("host-lifecycle", recorded), mk("host-lint", recorded)]),
            1,
            "a drifted host-lint submodule hazards"
        );
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0051 finding 3 / plan/0052: a materialized worktree moved OFF its pin must be
    // detected (DetectOffPin) and make the check HAZARDED (VerdictHazarded,
    // HazardBlocksClean, RecordHazard) — observable now that software_check returns its
    // hazard count. This is the test the off-pin and hazarded-verdict obligations were
    // mis-pointed away from (a pure helper that never drove the gate).
    #[test]
    fn off_pin_worktree_hazards_the_check() {
        let base = std::env::temp_dir().join(format!("hl-offpin-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git_ok(cwd, args), "git {args:?} failed");
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("readme.txt"), "seed").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "seed"]);
        let pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        let recipe = vec![Software {
            name: "demo".to_string(),
            url: src.to_string_lossy().to_string(),
            pin: pin.clone(),
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: Vec::new(),
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        software_materialize(&host, &recipe, false);
        let canon = host.join("software").join("demo").join("main");
        // At the pin: clean (the rule's precondition is not met — DetectOffPin.1 failure).
        assert_eq!(software_check(&host, &recipe), 0, "a matching pin is clean");
        // Move the worktree off its pin by committing ahead of it.
        g(&canon, &["config", "user.email", "t@t"]);
        g(&canon, &["config", "user.name", "t"]);
        fs::write(canon.join("readme.txt"), "drifted").unwrap();
        g(&canon, &["add", "-A"]);
        g(&canon, &["commit", "-qm", "drift"]);
        // HEAD != pin: DetectOffPin fires, the hazard blocks a clean verdict.
        assert!(software_check(&host, &recipe) > 0, "an off-pin worktree must hazard the check");
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0051 finding 4 / plan/0052: DetectUnreproducedArtifact. A rebuild whose hash
    // differs from the recorded artifact is detected, not silently accepted. The container
    // rebuild is integration-tested by --verify-build; this unit-tests the verdict so a
    // regression that accepted a non-matching rebuild cannot pass the suite green.
    #[test]
    fn unreproduced_artifact_is_detected() {
        assert!(artifact_reproduces(Some("aaaa"), "aaaa").is_ok(), "a matching rebuild reproduces");
        assert!(artifact_reproduces(Some("aaaa"), "bbbb").is_err(), "a non-matching rebuild must be detected");
        assert!(artifact_reproduces(None, "aaaa").is_err(), "a missing artifact must be an error");
    }

    // plan/0029: `--teardown` removes a clean, fully-pushed component (it re-materializes
    // from url + pin), and the unsaved-work guard flags a worktree that holds work.
    #[test]
    fn teardown_removes_clean_component() {
        let base = std::env::temp_dir().join(format!("hl-teardown-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git_ok(cwd, args), "git {args:?} failed");
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("readme.txt"), "seed").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "seed"]);
        let pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        let recipe = vec![Software {
            name: "demo".to_string(),
            url: src.to_string_lossy().to_string(),
            pin,
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: Vec::new(),
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        software_materialize(&host, &recipe, false);
        let comp = host.join("software").join("demo");
        assert!(comp.is_dir(), "materialized");
        // canonical worktree is at the pin (== origin/main): clean and pushed
        assert!(worktree_unsaved(&comp.join("main")).is_none(), "clean worktree is safe to remove");
        software_teardown(&host, &recipe, false);
        assert!(!comp.exists(), "component dir removed by teardown");
        let _ = fs::remove_dir_all(&base);
    }

    // The guard: an uncommitted change makes a worktree unsafe to tear down.
    #[test]
    fn teardown_guard_flags_uncommitted_changes() {
        let base = std::env::temp_dir().join(format!("hl-teardown-dirty-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let host = base.join("host");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&host).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git_ok(cwd, args), "git {args:?} failed");
        g(&src, &["init", "-q", "-b", "main"]);
        g(&src, &["config", "user.email", "t@t"]);
        g(&src, &["config", "user.name", "t"]);
        fs::write(src.join("readme.txt"), "seed").unwrap();
        g(&src, &["add", "-A"]);
        g(&src, &["commit", "-qm", "seed"]);
        let pin = git_out(&src, &["rev-parse", "HEAD"]).unwrap();
        let recipe = vec![Software {
            name: "demo".to_string(),
            url: src.to_string_lossy().to_string(),
            pin,
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: Vec::new(),
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        software_materialize(&host, &recipe, false);
        let canon = host.join("software").join("demo").join("main");
        fs::write(canon.join("scratch.txt"), "uncommitted").unwrap();
        assert!(worktree_unsaved(&canon).is_some(), "uncommitted change makes teardown unsafe");
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0029 robustness: branches colliding as a path (case-folding) or in git's
    // worktree admin (shared ref leaf) are a recipe defect software --check HAZARDs.
    #[test]
    fn branch_collisions_are_detected() {
        let mk = |worktrees: Vec<String>| Software {
            name: "demo".to_string(),
            url: "u".to_string(),
            pin: "p".to_string(),
            branch: "main".to_string(),
            worktrees,
            lines: Vec::new(),
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        };
        // case-insensitive collision with the canonical `main`
        assert_eq!(branch_collision_problems(&mk(vec!["Main".to_string()])).len(), 1);
        // worktree-admin leaf collision
        assert_eq!(
            branch_collision_problems(&mk(vec!["feature/login".to_string(), "bugfix/login".to_string()])).len(),
            1
        );
        // distinct branches: no collision
        assert!(branch_collision_problems(&mk(vec!["dev".to_string(), "release/2.0".to_string()])).is_empty());
    }

    // plan/0030 D4: the in-process prose audit mirrors host-lint --docs via the shared
    // scan_prose_text engine — a clean doc yields no warn/flag, and a decoration trope (an
    // em dash) yields a Warn the verify recheck treats as a regression.
    #[test]
    fn prose_audit_flags_a_trope_and_clears_clean_docs() {
        let base = std::env::temp_dir().join(format!("hl-prose-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let g = |args: &[&str]| assert!(git_ok(&base, args), "git {args:?} failed");
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        fs::write(base.join("clean.md"), "# Title\n\nThis document is plain authored prose.\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "clean"]);
        let clean = prose_audit(&base).unwrap();
        assert!(
            !clean.iter().any(|m| m.severity == Severity::Warn || m.severity == Severity::Flag),
            "a clean doc yields no warn/flag prose tropes"
        );
        // A decoration trope (an em dash used as a dramatic pause).
        fs::write(base.join("bad.md"), "# Title\n\nWe shipped the feature \u{2014} and it works.\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "bad"]);
        let dirty = prose_audit(&base).unwrap();
        assert!(
            dirty.iter().any(|m| m.severity == Severity::Warn || m.severity == Severity::Flag),
            "a decoration trope is detected as warn/flag"
        );
        let _ = fs::remove_dir_all(&base);
    }

    // host-lifecycle#2: the in-process prose audit now runs host-lint's shared --docs
    // engine, so it consults the repo's LEXICON. A declared domain noun clears the
    // ai-diction trope here exactly as at the CLI; before the shared-engine bump the
    // embedded engine ignored LEXICON and the verify recheck warned forever.
    #[test]
    fn prose_audit_honours_lexicon_masking() {
        let base = std::env::temp_dir().join(format!("hl-prose-lex-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let g = |args: &[&str]| assert!(git_ok(&base, args), "git {args:?} failed");
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        // "harness" is an ai-diction term; two occurrences trip the density warn.
        fs::write(
            base.join("doc.md"),
            "# Title\n\nThe wdm-harness drives the lane. The harness emits a verdict.\n",
        )
        .unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "doc"]);
        // No LEXICON: the prose tell warns.
        let bare = prose_audit(&base).unwrap();
        assert!(
            bare.iter().any(|m| m.severity == Severity::Warn),
            "undeclared, the ai-diction term warns in the in-process audit"
        );
        // Declare the phrases: the audit masks them before detection, so the warn clears.
        fs::write(base.join("LEXICON"), "wdm-harness\nthe harness\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "lexicon"]);
        let masked = prose_audit(&base).unwrap();
        assert!(
            !masked.iter().any(|m| m.severity == Severity::Warn),
            "a LEXICON-declared phrase clears the prose tell in the in-process audit"
        );
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0036 + plan/0039: the reconcile arm. A project's own truth parses out of its
    // `.host-software` (components minus the entrance; verifier drivers); an annotation
    // names a kind; an assertion flags a drifted restatement and clears a matching one;
    // the ledger `restates` field parses and gates its kinds. The spine manifest is
    // phases only — `manifest --check` rejects a project-fact stanza.
    #[test]
    fn parse_project_facts_reads_components_minus_entrance_and_drivers() {
        // the [entrance] stanza: host is set apart, the four tools are the components, the
        // document and the restated concepts parse, and there are no problems.
        let text = "[software \"host-lint\"]\n\turl = u\n[software \"host-lifecycle\"]\n\turl = u\n[software \"host-prove\"]\n\turl = u\n[software \"host-grammar\"]\n\turl = u\n[software \"host\"]\n\turl = u\n\n[entrance]\n\tmember = host\n\tdocument = README.md\n\trestates = phases tools\n\n[verification]\n\tdrivers = host-lint allium specula host-prove\n";
        let f = parse_project_facts(text);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        assert_eq!(f.components, vec!["host-lint", "host-lifecycle", "host-prove", "host-grammar"]);
        assert_eq!(f.drivers, vec!["host-lint", "allium", "specula", "host-prove"]);
        let e = f.entrance.expect("an entrance");
        assert_eq!((e.member.as_str(), e.document.as_str()), ("host", "README.md"));
        assert!(e.restates.checks("phases") && e.restates.checks("tools") && !e.restates.checks("stamp"));

        // `restates` defaults to every concept and `document` to README.md when omitted.
        let bare = parse_project_facts("[software \"host\"]\n\turl = u\n\n[entrance]\n\tmember = host\n");
        let be = bare.entrance.expect("an entrance");
        assert_eq!(be.document, "README.md");
        assert!(be.restates.checks("stamp"), "an omitted restates means every concept");

        // a member that forgets every marker is counted a component (fail-safe)
        let unmarked = "[software \"host-lint\"]\n\turl = u\n[software \"host\"]\n\turl = u\n";
        let u = parse_project_facts(unmarked);
        assert_eq!(u.components, vec!["host-lint", "host"]);
        assert!(u.entrance.is_none());

        // the legacy per-member marker is retired: it is a problem, not the entrance
        let legacy = "[software \"host-lint\"]\n\turl = u\n[software \"host\"]\n\turl = u\n\tfront-door = true\n";
        let lf = parse_project_facts(legacy);
        assert!(lf.problems.iter().any(|p| p.contains("retired")), "{:?}", lf.problems);
        assert!(lf.entrance.is_none());

        // a second [entrance] stanza is a singleton problem
        let two = parse_project_facts("[software \"host\"]\n\turl = u\n[entrance]\n\tmember = host\n[entrance]\n\tmember = host\n");
        assert!(two.problems.iter().any(|p| p.contains("singleton")), "{:?}", two.problems);
        // an unknown restated concept is a problem
        let bad = parse_project_facts("[software \"host\"]\n\turl = u\n[entrance]\n\tmember = host\n\trestates = phases bogus\n");
        assert!(bad.problems.iter().any(|p| p.contains("bogus")), "{:?}", bad.problems);
        // a named member that is not declared is a problem
        let nomember = parse_project_facts("[software \"host\"]\n\turl = u\n[entrance]\n\tmember = nope\n");
        assert!(nomember.problems.iter().any(|p| p.contains("not a declared")), "{:?}", nomember.problems);
        // an empty `restates` value is a problem (not a silent check-nothing)
        let empty = parse_project_facts("[software \"host\"]\n\turl = u\n[entrance]\n\tmember = host\n\trestates =\n");
        assert!(empty.problems.iter().any(|p| p.contains("empty")), "{:?}", empty.problems);
        // a sub-named `[entrance \"x\"]` is the wrong form, flagged but still parsed
        let subnamed = parse_project_facts("[software \"host\"]\n\turl = u\n[entrance \"host\"]\n\tmember = host\n");
        assert!(subnamed.problems.iter().any(|p| p.contains("sub-named")), "{:?}", subnamed.problems);
        assert_eq!(subnamed.entrance.as_ref().map(|e| e.member.as_str()), Some("host"));
        // a `document` that escapes the worktree is a problem
        let escape = parse_project_facts("[software \"host\"]\n\turl = u\n[entrance]\n\tmember = host\n\tdocument = ../etc/passwd\n");
        assert!(escape.problems.iter().any(|p| p.contains("within the member")), "{:?}", escape.problems);
        // the retired marker is caught case-insensitively, so a `True` is a problem too
        let cased = parse_project_facts("[software \"host-lint\"]\n\turl = u\n[software \"host\"]\n\turl = u\n\tentrance = True\n");
        assert!(cased.problems.iter().any(|p| p.contains("retired")), "{:?}", cased.problems);
        assert!(cased.entrance.is_none());
        // a duplicate `member` is a problem
        let dup = parse_project_facts("[software \"a\"]\n\turl = u\n[software \"b\"]\n\turl = u\n[entrance]\n\tmember = a\n\tmember = b\n");
        assert!(dup.problems.iter().any(|p| p.contains("more than once")), "{:?}", dup.problems);
        // issue #14: [verification] is a singleton, like [entrance]. A second stanza, a repeated
        // `drivers` key (last-wins), or an empty value silently disarms the coverage check.
        let two_v = parse_project_facts("[software \"host\"]\n\turl = u\n[verification]\n\tdrivers = host-lint\n[verification]\n\tdrivers = allium\n");
        assert!(two_v.problems.iter().any(|p| p.contains("singleton") && p.contains("verification")), "{:?}", two_v.problems);
        let dup_drivers = parse_project_facts("[software \"host\"]\n\turl = u\n[verification]\n\tdrivers = host-lint\n\tdrivers = allium\n");
        assert!(dup_drivers.problems.iter().any(|p| p.contains("drivers") && p.contains("more than once")), "{:?}", dup_drivers.problems);
        let empty_drivers = parse_project_facts("[software \"host\"]\n\turl = u\n[verification]\n\tdrivers =\n");
        assert!(empty_drivers.problems.iter().any(|p| p.contains("drivers") && p.contains("empty")), "{:?}", empty_drivers.problems);
    }

    // issue #6: ASCII quotes an operator wraps around value tokens are stripped in this twin parser
    // too — the entrance member/document and the drivers list — never leaked downstream. Reverting
    // the unquote calls fails here: `document = "README.md"` would keep the quotes and pass the
    // escape check, and `member = "host"` would not match the declared member.
    #[test]
    fn parse_project_facts_strips_value_quotes() {
        let text = "[software \"host\"]\n\turl = u\n[entrance]\n\tmember = \"host\"\n\tdocument = \"README.md\"\n\trestates = \"phases\" \"tools\"\n[verification]\n\tdrivers = \"host-lint\" \"allium\"\n";
        let f = parse_project_facts(text);
        assert!(f.problems.is_empty(), "quoted values parse cleanly: {:?}", f.problems);
        assert_eq!(f.drivers, vec!["host-lint", "allium"]);
        let e = f.entrance.expect("an entrance");
        assert_eq!((e.member.as_str(), e.document.as_str()), ("host", "README.md"));
        assert!(e.restates.checks("phases") && e.restates.checks("tools"));
    }

    #[test]
    fn manifest_check_rejects_non_phase_stanzas() {
        let ok = "[phase \"verify\"]\n\torder = 5\n";
        assert!(manifest_foreign_stanzas(ok).is_empty(), "a phases-only manifest is clean");
        let bad = "[phase \"verify\"]\n\torder = 5\n\n[components]\n\ttools = host-lint\n[verification]\n\tdrivers = host-lint\n";
        let f = manifest_foreign_stanzas(bad);
        assert_eq!(f.len(), 2, "both [components] and [verification] are foreign: {f:?}");
        assert!(f[0].1.contains("[components]") && f[1].1.contains("[verification]"));
    }

    #[test]
    fn reconcile_kind_extracts_the_annotation() {
        assert_eq!(reconcile_kind("text here <!-- host-reconcile: components -->").as_deref(), Some("components"));
        assert_eq!(reconcile_kind("| Where | software/ | <!-- host-reconcile: where-root -->").as_deref(), Some("where-root"));
        assert_eq!(reconcile_kind("plain line, no annotation"), None);
        assert_eq!(reconcile_kind("<!-- host-reconcile:  -->"), None);
    }

    #[test]
    fn plan_index_problems_flags_unindexed_ghost_and_ignores_prose_and_fences() {
        let doc = |r: &str, c: &str| (r.to_string(), c.to_string());
        // gap: plan/0002-y has a tracked README, PLAN.md indexes only 0001
        let gap = vec![
            doc("PLAN.md", "# Plan\n| [x](plan/0001-x/README.md) | done |\n"),
            doc("plan/0001-x/README.md", "x"),
            doc("plan/0002-y/README.md", "y"),
        ];
        assert!(
            plan_index_problems(&gap).iter().any(|h| h.contains("omits") && h.contains("plan/0002-y")),
            "an unindexed directory hazards: {:?}",
            plan_index_problems(&gap)
        );
        // clean: both directories linked
        let clean = vec![
            doc("PLAN.md", "# Plan\n| [x](plan/0001-x/README.md) |\n| [y](plan/0002-y/README.md) |\n"),
            doc("plan/0001-x/README.md", "x"),
            doc("plan/0002-y/README.md", "y"),
        ];
        assert!(plan_index_problems(&clean).is_empty(), "a fully-indexed set is clean: {:?}", plan_index_problems(&clean));
        // a bare prose mention is not a live row
        let prose = vec![
            doc("PLAN.md", "# Plan\n| [x](plan/0001-x/README.md) |\nplan/0002-y is cut but carries no linked row\n"),
            doc("plan/0001-x/README.md", "x"),
            doc("plan/0002-y/README.md", "y"),
        ];
        assert!(plan_index_problems(&prose).iter().any(|h| h.contains("plan/0002-y")), "a prose mention is not a row");
        // reverse: a row links a directory that does not exist
        let ghost = vec![
            doc("PLAN.md", "# Plan\n| [x](plan/0001-x/README.md) |\n| [g](plan/0099-ghost/README.md) |\n"),
            doc("plan/0001-x/README.md", "x"),
        ];
        assert!(
            plan_index_problems(&ghost).iter().any(|h| h.contains("no such plan/ directory") && h.contains("plan/0099-ghost")),
            "a row linking a nonexistent directory hazards"
        );
        // a link inside a fenced block is masked out, so it is not a live row
        let fenced = vec![
            doc("PLAN.md", "# Plan\n| [x](plan/0001-x/README.md) |\n```\n[ex](plan/0002-y/README.md)\n```\n"),
            doc("plan/0001-x/README.md", "x"),
            doc("plan/0002-y/README.md", "y"),
        ];
        assert!(plan_index_problems(&fenced).iter().any(|h| h.contains("plan/0002-y")), "a fenced link is not a live row");
        // no PLAN.md: inert
        assert!(plan_index_problems(&[doc("plan/0001-x/README.md", "x")]).is_empty(), "no PLAN.md is inert");
    }

    #[test]
    fn reconcile_assertion_flags_drift_and_clears_clean() {
        let facts = ProjectFacts {
            components: vec!["host-lint".into(), "host-lifecycle".into(), "host-prove".into(), "host-grammar".into()],
            drivers: vec!["host-lint".into(), "allium".into(), "specula".into(), "host-prove".into()],
            ..Default::default()
        };
        // components: a line that omits host-prove flags; the full set clears.
        assert!(reconcile_assertion("components", "the host-* components (host-grammar, host-lint, host-lifecycle)", &facts).is_some());
        assert!(reconcile_assertion("components", "host-lint, host-lifecycle, host-prove, host-grammar", &facts).is_none());
        // verification: "three lanes" dropping host-prove flags (case-insensitive).
        assert!(reconcile_assertion("verification", "three lanes: host-lint, allium, Specula", &facts).is_some());
        assert!(reconcile_assertion("verification", "host-lint, allium, specula, and host-prove", &facts).is_none());
        // where-root: a stale root flags; naming software/ clears.
        assert!(reconcile_assertion("where-root", "Where at host-lint/", &facts).is_some());
        assert!(reconcile_assertion("where-root", "Where at software/<name>/main/", &facts).is_none());
        // spec-path: specs placed under plan/ flags; co-located clears.
        assert!(reconcile_assertion("spec-path", "specifications at plan/<NNNN>/spec/", &facts).is_some());
        assert!(reconcile_assertion("spec-path", "specs co-locate with the software in its repo", &facts).is_none());
        // an unknown kind is itself a HAZARD (guards an annotation typo).
        assert!(reconcile_assertion("bogus", "anything", &facts).is_some());
        // empty datum (pre-data template): components/verification unverifiable, skipped.
        let bare = ProjectFacts { components: vec![], drivers: vec![], ..Default::default() };
        assert!(reconcile_assertion("components", "host-lint only", &bare).is_none());
    }

    #[test]
    fn parse_upgrading_reads_restates_and_ledger_gates_kinds() {
        let text = "[upgrade \"abc1234\"]\n\ttitle = t\n\trestates = components verification\n\n[upgrade \"def5678\"]\n\ttitle = t2\n\trestates = bogus-kind\n";
        let entries = parse_upgrading(text);
        assert_eq!(entries[0].restates, vec!["components", "verification"]);
        let problems = validate_ledger(&entries);
        let restate_problems: Vec<&String> = problems.iter().filter(|p| p.contains("restates unknown kind")).collect();
        assert_eq!(restate_problems.len(), 1, "only the bogus kind is flagged: {problems:?}");
        assert!(restate_problems[0].contains("`bogus-kind`"), "the flagged kind is bogus-kind: {restate_problems:?}");
    }

    #[test]
    fn reconcile_scan_flags_an_annotated_drift_and_clears_a_clean_doc() {
        let base = std::env::temp_dir().join(format!("hl-reconcile-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let g = |args: &[&str]| assert!(git_ok(&base, args), "git {args:?} failed");
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        let facts = ProjectFacts {
            components: vec!["host-lint".into(), "host-lifecycle".into(), "host-prove".into(), "host-grammar".into()],
            drivers: vec!["host-lint".into(), "allium".into(), "specula".into(), "host-prove".into()],
            ..Default::default()
        };
        // a clean annotated restatement: no hazard
        fs::write(base.join("ok.md"), "Develops host-lint, host-lifecycle, host-prove, host-grammar. <!-- host-reconcile: components -->\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "ok"]);
        assert!(reconcile_scan(&tracked_markdown(&base).unwrap(), &facts).is_empty(), "a clean annotated doc yields no hazard");
        // a drifted restatement (omits host-prove): one hazard at the annotated line
        fs::write(base.join("drift.md"), "# T\n\nThe host-* tooling (host-grammar, host-lint, host-lifecycle). <!-- host-reconcile: components -->\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "drift"]);
        let hz = reconcile_scan(&tracked_markdown(&base).unwrap(), &facts);
        assert_eq!(hz.len(), 1, "exactly the drifted restatement flags: {hz:?}");
        assert!(hz[0].contains("drift.md:3") && hz[0].contains("host-prove"), "hazard names file:line and the omission: {hz:?}");
        // a doc that QUOTES the marker in inline code is documentation, not a directive:
        // the placeholder kind would be unknown, but the backtick span suppresses it (the
        // spine and every case-(a) adopter carry this example in their CLAUDE.md).
        fs::write(base.join("ok.md"), "Develops host-lint, host-lifecycle, host-prove, host-grammar. <!-- host-reconcile: components -->\nIt carries an inline `<!-- host-reconcile: KIND -->` annotation.\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "doc"]);
        assert_eq!(reconcile_scan(&tracked_markdown(&base).unwrap(), &facts).len(), 1, "the backtick-quoted example adds no hazard; only the real drift remains");
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0039: the concept-as-URI checks. Definitions live at `{#id}` anchors in an
    // authored doc; pointers link to them; coverage holds each project-local home to its
    // full `.host-software` set. (Pure over in-memory docs — no temp git repo needed.)
    #[test]
    fn concept_checks_cover_links_homes_and_membership() {
        let facts = ProjectFacts {
            components: vec!["host-lint".into(), "host-lifecycle".into(), "host-prove".into(), "host-grammar".into()],
            drivers: vec!["host-lint".into(), "allium".into(), "specula".into(), "host-prove".into()],
            ..Default::default()
        };
        let structure = |components_line: &str| {
            format!("## Components {{#components}}\n{components_line}\n\n## Verifiers {{#verifiers}}\nhost-lint, allium, specula, host-prove.\n")
        };
        // clean: each home names its full set; the pointers resolve (caps file part).
        let clean = vec![
            ("STRUCTURE.md".to_string(), structure("host-lint, host-lifecycle, host-prove, host-grammar.")),
            ("README.md".to_string(), "Built on the [components](STRUCTURE.md#components) and [verifiers](STRUCTURE.md#verifiers).\n".to_string()),
        ];
        assert!(concept_checks(&clean, &facts).is_empty(), "a clean repo yields no hazard: {:?}", concept_checks(&clean, &facts));
        // coverage bite: the components home drops host-prove.
        let drift = vec![("STRUCTURE.md".to_string(), structure("host-lint, host-lifecycle, host-grammar."))];
        assert!(concept_checks(&drift, &facts).iter().any(|m| m.contains("omits") && m.contains("host-prove")), "coverage flags the dropped member");
        // declared-anchor: a pointer to a concept with no home anywhere.
        let nohome = vec![("README.md".to_string(), "See the [components](STRUCTURE.md#components).\n".to_string())];
        assert!(concept_checks(&nohome, &facts).iter().any(|m| m.contains("no doc defines that concept")), "missing home flags");
        // link-integrity: a lowercased file part does not resolve to the caps home.
        let badcase = vec![
            ("STRUCTURE.md".to_string(), structure("host-lint, host-lifecycle, host-prove, host-grammar.")),
            ("README.md".to_string(), "See the [components](structure.md#components).\n".to_string()),
        ];
        assert!(concept_checks(&badcase, &facts).iter().any(|m| m.contains("does not resolve")), "wrong-case file part flags");
        // one-home rule: the same concept defined twice is ambiguous.
        let dup = vec![
            ("STRUCTURE.md".to_string(), structure("host-lint, host-lifecycle, host-prove, host-grammar.")),
            ("OTHER.md".to_string(), "## Components {#components}\nx.\n".to_string()),
        ];
        assert!(concept_checks(&dup, &facts).iter().any(|m| m.contains("more than one place")), "ambiguous home flags");
        // a {#id} on a non-heading line is not a home (mdBook renders no anchor there),
        // so a pointer to it fails declared-anchor rather than passing falsely.
        let nonheading = vec![
            ("STRUCTURE.md".to_string(), "Components {#components}: host-lint, host-lifecycle, host-prove, host-grammar.\n".to_string()),
            ("README.md".to_string(), "See the [components](STRUCTURE.md#components).\n".to_string()),
        ];
        assert!(concept_checks(&nonheading, &facts).iter().any(|m| m.contains("no doc defines that concept")), "a non-heading anchor is not a home");
        // a {#id} at the START of a heading (`## {#id} Title`) is slugified by mdBook to a
        // different id, so it is not a home either; a pointer to it fails declared-anchor.
        let startplaced = vec![
            ("STRUCTURE.md".to_string(), "## {#components} Components\nhost-lint, host-lifecycle, host-prove, host-grammar.\n".to_string()),
            ("README.md".to_string(), "See the [components](STRUCTURE.md#components).\n".to_string()),
        ];
        assert!(concept_checks(&startplaced, &facts).iter().any(|m| m.contains("no doc defines that concept")), "a start-placed anchor is not a home");
        // a home whose members live in deeper sub-headings (## Components / ### each-tool)
        // still covers: the section runs to the next same-or-higher heading.
        let subheaded = vec![(
            "STRUCTURE.md".to_string(),
            "## Components {#components}\n\n### host-lint\nx\n\n### host-lifecycle\nx\n\n### host-prove\nx\n\n### host-grammar\nx\n\n## Next section\nunrelated\n".to_string(),
        )];
        assert!(concept_checks(&subheaded, &facts).is_empty(), "a sub-headed home covers all members: {:?}", concept_checks(&subheaded, &facts));
        // issue #12: a heading carrying {#components} inside a fenced example is not a live home,
        // so the real (unfenced) pointer fails declared-anchor rather than resolving to it.
        let fenced_home = vec![
            ("STRUCTURE.md".to_string(), "Example:\n\n```\n## Components {#components}\nfoo\n```\n".to_string()),
            ("README.md".to_string(), "See the [components](STRUCTURE.md#components).\n".to_string()),
        ];
        assert!(concept_checks(&fenced_home, &facts).iter().any(|m| m.contains("no doc defines that concept")), "a fenced anchor is not a home (issue #12)");
        // issue #12: a concept link inside a fenced example is not a live link, so a deliberately
        // wrong file part in a fence does not flag link-integrity.
        let fenced_link = vec![
            ("STRUCTURE.md".to_string(), structure("host-lint, host-lifecycle, host-prove, host-grammar.")),
            ("README.md".to_string(), "Real: [components](STRUCTURE.md#components).\n\n```\n[components](wrong.md#components)\n```\n".to_string()),
        ];
        assert!(concept_checks(&fenced_link, &facts).is_empty(), "a fenced link is not checked (issue #12): {:?}", concept_checks(&fenced_link, &facts));
        // issue #19: a home that names only a longer identifier (host-prove-helper) does not cover
        // the member host-prove — a substring check would miss this omission.
        let substring_only = vec![("STRUCTURE.md".to_string(), structure("host-lint, host-lifecycle, host-prove-helper, host-grammar."))];
        assert!(concept_checks(&substring_only, &facts).iter().any(|m| m.contains("omits") && m.contains("host-prove")), "token-boundary coverage flags host-prove (issue #19)");
        // issue #15: the single-value homes carry a content bite too. The canonical wording is
        // clean; a software-root home that drops `software/`, or a spec-home that drops the
        // co-location affirmation, is a HAZARD (and the canonical "never under `plan/`" must NOT
        // false-positive the way the old `plan/`-and-`spec` predicate would).
        let single = |sw: &str, spec: &str| {
            vec![(
                "STRUCTURE.md".to_string(),
                format!("## Software-root {{#software-root}}\n{sw}\n\n## Spec-home {{#spec-home}}\n{spec}\n"),
            )]
        };
        let canonical = single(
            "Where the software under development lives: `software/`.",
            "Where specifications live: with their software, co-located in each component's repo, never under `plan/`.",
        );
        assert!(concept_checks(&canonical, &facts).is_empty(), "the canonical single-value homes are clean: {:?}", concept_checks(&canonical, &facts));
        let sw_drift = single("Where the software lives: under the root.", "co-located with their software.");
        assert!(concept_checks(&sw_drift, &facts).iter().any(|m| m.contains("software-root")), "a software-root home dropping `software/` flags (issue #15)");
        let spec_drift = single("lives under `software/`.", "Specs live under `plan/<milestone>/spec/`.");
        assert!(concept_checks(&spec_drift, &facts).iter().any(|m| m.contains("spec-home")), "a spec-home home dropping co-location flags (issue #15)");
    }

    // plan/0037: migrate-receipts moves the applied-set into .host-receipts and splits the
    // operational receipts into .host-lifecycle-receipts, the dual-format reads still see
    // everything, and a second run is a no-op.
    #[test]
    fn migrate_receipts_moves_applied_and_splits_operational() {
        let base = std::env::temp_dir().join(format!("hl-migrate-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(
            base.join(".host"),
            "template = \"x\"\nrevision = \"a5fef9d\"\nbaseline = \"a22704e\"\napplied = 897ce0d recorded=2026-06-20 via=verify\napplied = da000aa recorded=2026-06-20 via=verify\n",
        ).unwrap();
        fs::write(
            base.join(".host-receipts"),
            "[receipt \"adopt\"]\n    disposition = done\n    evidence = a5fef9d\n\n[receipt \"embed\" \"host-lint\"]\n    disposition = done\n    evidence = pin abc\n\n[receipt \"release\" \"host-lint\"]\n    disposition = done\n    evidence = v1@h\n\n[receipt \"upgrade\"]\n    disposition = done\n    evidence = ledger\n",
        ).unwrap();
        let arg = base.to_string_lossy().to_string();
        migrate_receipts(std::slice::from_ref(&arg));
        let canon = fs::canonicalize(&base).unwrap();

        let stamp = fs::read_to_string(canon.join(".host")).unwrap();
        assert!(!stamp.contains("applied ="), ".host no longer holds the applied-set");
        assert!(stamp.contains("baseline = \"a22704e\""), "baseline stays in .host");
        let rc = fs::read_to_string(canon.join(".host-receipts")).unwrap();
        assert!(rc.contains("applied = 897ce0d") && rc.contains("applied = da000aa"), "applied-set moved into .host-receipts");
        assert!(rc.contains("[receipt \"adopt\"]") && rc.contains("[receipt \"upgrade\"]"), "methodology receipts stay");
        assert!(!rc.contains("[receipt \"embed\"") && !rc.contains("[receipt \"release\""), "operational receipts left .host-receipts");
        let op = fs::read_to_string(canon.join(".host-lifecycle-receipts")).unwrap();
        assert!(op.contains("[receipt \"embed\" \"host-lint\"]") && op.contains("[receipt \"release\" \"host-lint\"]"), "operational receipts moved here");

        // dual-format reads see the whole picture
        let ids = read_applied_ids(&canon);
        assert!(ids.contains(&"897ce0d".to_string()) && ids.contains(&"da000aa".to_string()), "applied ids read from the new layout");
        let all = read_all_receipts(&canon);
        assert!(all.iter().any(|r| r.phase == "adopt") && all.iter().any(|r| r.phase == "embed") && all.iter().any(|r| r.phase == "release"), "gate unions both receipt files");

        // idempotent
        migrate_receipts(std::slice::from_ref(&arg));
        let rc2 = fs::read_to_string(canon.join(".host-receipts")).unwrap();
        assert!(rc2.contains("applied = 897ce0d") && !rc2.contains("[receipt \"embed\""), "second run is a no-op");
        let _ = fs::remove_dir_all(&base);
    }

    // issue #9: a crash can leave the applied set in BOTH .host and .host-receipts (the new
    // write order writes the receiver before the source sheds). The recovery re-run must
    // converge — applied set in .host-receipts exactly once, stripped from .host — never lost
    // to the data-shedding-first order, never doubled.
    #[test]
    fn migrate_receipts_recovers_a_partial_write_without_loss_or_duplication() {
        let base = std::env::temp_dir().join(format!("hl-migrate-recover-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let applied = "applied = 897ce0d recorded=2026-06-20 via=verify";
        // .host still carries the applied line (the shed never completed); .host-receipts
        // already received it alongside a methodology receipt.
        fs::write(base.join(".host"), format!("baseline = \"a22704e\"\n{applied}\n")).unwrap();
        fs::write(base.join(".host-receipts"), format!("{applied}\n\n[receipt \"adopt\"]\n    disposition = done\n    evidence = a5fef9d\n")).unwrap();
        let arg = base.to_string_lossy().to_string();
        migrate_receipts(std::slice::from_ref(&arg));
        let canon = fs::canonicalize(&base).unwrap();
        let stamp = fs::read_to_string(canon.join(".host")).unwrap();
        let rc = fs::read_to_string(canon.join(".host-receipts")).unwrap();
        assert!(!stamp.contains("applied ="), ".host shed the applied set");
        assert_eq!(rc.matches("applied = 897ce0d").count(), 1, "applied set present exactly once (deduped)");
        assert!(rc.contains("[receipt \"adopt\"]"), "the methodology receipt survives");
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0037: a legacy layout (applied-set in .host, a single .host-receipts, no
    // .host-lifecycle-receipts) is read correctly without migrating (auto-migrate on read).
    #[test]
    fn reads_legacy_layout_without_migrating() {
        let base = std::env::temp_dir().join(format!("hl-legacy-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join(".host"), "baseline = \"a22704e\"\napplied = 897ce0d recorded=2026-06-20 via=verify\n").unwrap();
        fs::write(base.join(".host-receipts"), "[receipt \"embed\" \"host-lint\"]\n    disposition = done\n    evidence = pin abc\n").unwrap();
        let canon = fs::canonicalize(&base).unwrap();
        assert_eq!(read_applied_ids(&canon), vec!["897ce0d".to_string()], "applied read from legacy .host");
        assert!(read_all_receipts(&canon).iter().any(|r| r.phase == "embed"), "receipts read with no .host-lifecycle-receipts present");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn normalize_join_resolves_dotdot() {
        assert_eq!(normalize_join(".claude/skills", "../../host-lint"), "host-lint");
        assert_eq!(normalize_join("a/b", "../c"), "a/c");
        assert_eq!(normalize_join("", "host-lint/SKILL.md"), "host-lint/SKILL.md");
        assert_eq!(normalize_join(".claude/skills", "../../host-lint/SKILL.md"), "host-lint/SKILL.md");
    }

    // A generated (untracked) skill link that dangles is flagged; a resolving one is
    // not (plan/0029). The tracked-symlink hazard cannot see these.
    #[cfg(unix)]
    #[test]
    fn dangling_generated_skill_links_are_flagged() {
        let base = std::env::temp_dir().join(format!("hl-genlink-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let skills = base.join(".claude").join("skills");
        fs::create_dir_all(&skills).unwrap();
        let target = base.join("real");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, skills.join("good")).unwrap();
        std::os::unix::fs::symlink(base.join("gone"), skills.join("bad")).unwrap();
        assert_eq!(dangling_generated_links(&base), vec!["bad".to_string()]);
        let _ = fs::remove_dir_all(&base);
    }

    // A tracked symlink whose target isn't tracked here (a worktree/submodule
    // sub-path) is a hazard; one pointing at a tracked file is not.
    #[cfg(unix)]
    #[test]
    fn flags_symlinks_into_untracked_paths_only() {
        let base = std::env::temp_dir().join(format!("hl-hazard-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".claude/skills")).unwrap();
        fs::create_dir_all(base.join("docs")).unwrap();
        let g = |args: &[&str]| assert!(git_ok(&base, args), "git {args:?}");
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        fs::write(base.join("README.md"), "doc").unwrap();
        // into an un-materialized path (a worktree/submodule sub-path) → hazard
        std::os::unix::fs::symlink("../../demo/skill", base.join(".claude/skills/demo")).unwrap();
        // into a tracked file → fine
        std::os::unix::fs::symlink("../README.md", base.join("docs/readme")).unwrap();
        g(&["add", "-A"]);

        let haz = dangling_symlink_hazards(&base);
        assert_eq!(haz.len(), 1, "only the symlink into an untracked path is a hazard");
        assert_eq!(haz[0].0, ".claude/skills/demo");
        assert_eq!(haz[0].1, "demo/skill");

        let _ = fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod upgrade_tests {
    use super::*;

    #[test]
    fn parses_upgrading_ledger() {
        let text = "\
# the ledger
[upgrade \"8c28e33\"]
    title    = Bare store with worktrees
    action   = Convert the embedded submodule.
    requires = host-lifecycle v0.3.0

[upgrade \"abc1234\"]
    title    = Worktree-absence coherence
    action   = Untrack worktree symlinks.
";
        let e = parse_upgrading(text);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].revision, "8c28e33");
        assert_eq!(e[0].title, "Bare store with worktrees");
        assert_eq!(e[0].requires, "host-lifecycle v0.3.0");
        assert_eq!(e[1].revision, "abc1234");
        assert!(e[1].requires.is_empty());
    }

}

#[cfg(test)]
mod book_tests {
    use super::*;

    #[test]
    fn first_heading_finds_title() {
        assert_eq!(first_heading("# Mara — operator\n\nbody"), Some("Mara — operator".to_string()));
        assert_eq!(first_heading("intro\n## sub\n# Real\n").as_deref(), Some("Real"));
        assert_eq!(first_heading("no heading here\n"), None);
        assert_eq!(first_heading("#nospace\n"), None);
    }

    #[test]
    fn humanize_replaces_separators() {
        assert_eq!(humanize("0001-migration-protocol"), "0001 migration protocol");
        assert_eq!(humanize("a_b-c"), "a b c");
    }

    #[test]
    fn book_toml_scopes_src_to_mdbook() {
        let base = std::env::temp_dir().join(format!("hl-toml-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let toml = book_toml(&base);
        // Generated source and HTML under one gitignored mdBook/ folder (host-lifecycle#3);
        // never src = "." (call/0005). book.toml stays at root, so mdbook builds from root.
        assert!(toml.contains("src = \"mdBook/src\""), "src scoped to mdBook/src");
        assert!(toml.contains("build-dir = \"mdBook/out\""), "build-dir scoped to mdBook/out");
        assert!(!toml.contains("src = \".\""));
        assert!(toml.contains("default-theme = \"light\""));
        // no custom.css → no additional-css line
        assert!(!toml.contains("additional-css"));
        fs::write(base.join("custom.css"), "body{}").unwrap();
        assert!(book_toml(&base).contains("additional-css = [\"custom.css\"]"));
        let _ = fs::remove_dir_all(&base);
    }

    // host-lifecycle#3: the generator writes its source under mdBook/src/ and never touches a
    // project's authored docs/, so a migrating project that keeps documentation in docs/ is safe.
    #[test]
    fn write_book_targets_mdbook_dir_and_leaves_docs_intact() {
        let base = std::env::temp_dir().join(format!("hl-writebook-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        // A pre-existing authored docs/ that must survive untouched.
        fs::create_dir_all(base.join("docs")).unwrap();
        fs::write(base.join("docs/authored.md"), "hand-written").unwrap();

        let home = Page {
            dest: "index.md".to_string(),
            label: "Home".to_string(),
            depth: 0,
            body: PageBody::Inline("# Home\n".to_string()),
        };
        let sections = vec![Section {
            title: "Cast: who".to_string(),
            room: "cast",
            required: true,
            pages: vec![Page {
                dest: "cast/mara.md".to_string(),
                label: "Mara".to_string(),
                depth: 1,
                body: PageBody::Inline("# Mara\n".to_string()),
            }],
        }];
        write_book(&base, &home, &sections, false);

        assert!(base.join("mdBook/src/SUMMARY.md").is_file(), "SUMMARY lands under mdBook/src");
        assert!(base.join("mdBook/src/cast/mara.md").is_file(), "pages land under mdBook/src");
        assert!(base.join("book.toml").is_file(), "book.toml stays at the root");
        assert!(book_toml(&base).contains("src = \"mdBook/src\""));
        // The regression: the authored docs/ is never read, cleared, or written.
        assert!(base.join("docs/authored.md").is_file(), "authored docs/ is left intact");
        assert!(!base.join("docs/SUMMARY.md").exists(), "nothing is generated into docs/");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn book_toml_title_comes_from_stamp_name() {
        let base = std::env::temp_dir().join(format!("hl-title-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        // no stamp → title is the directory basename
        let dir = base.file_name().unwrap().to_string_lossy().to_string();
        assert!(book_toml(&base).contains(&format!("title = \"{dir}\"")));
        // stamp `name` pins the title deterministically, regardless of dir name
        fs::write(base.join(STAMP), "template = \"x\"\nrevision = \"abc\"\nname = \"agentic-host\"\n").unwrap();
        assert!(book_toml(&base).contains("title = \"agentic-host\""));
        assert!(!book_toml(&base).contains(&format!("title = \"{dir}\"")));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn book_toml_emits_site_url_only_for_a_non_default_mount() {
        let base = std::env::temp_dir().join(format!("hl-mount-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        // no book-mount declared -> default "/", and book.toml carries no site-url line
        fs::write(base.join(STAMP), "template = \"x\"\nrevision = \"abc\"\nname = \"proj\"\n").unwrap();
        assert_eq!(stamp_book_mount(&base), "/");
        assert!(!book_toml(&base).contains("site-url"), "the default mount emits no site-url");
        // a declared sub-path mount -> mdBook site-url under [output.html]
        fs::write(base.join(STAMP), "template = \"x\"\nrevision = \"abc\"\nname = \"proj\"\nbook-mount = \"/book/\"\n").unwrap();
        assert_eq!(stamp_book_mount(&base), "/book/");
        assert!(book_toml(&base).contains("site-url = \"/book/\""), "a non-default mount emits site-url: {}", book_toml(&base));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn home_page_prefers_readme_else_generates_overview() {
        let base = std::env::temp_dir().join(format!("hl-home-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("cast")).unwrap();
        fs::write(base.join("cast/README.md"), "# Cast\n").unwrap();
        let sections = plan_book(&base);
        // no README/home.md → generated overview linking the rooms, titled by name
        let gen = home_page(&base, "proj", &sections);
        assert_eq!(gen.dest, "index.md");
        match &gen.body {
            PageBody::Inline(t) => {
                assert!(t.starts_with("# proj\n"));
                // host#15: link the served index page, never `cast/README.md` (mdBook
                // rewrites that in-content link to a non-existent `cast/README.html`).
                assert!(t.contains("](cast/index.md)"), "links the served Cast index");
                assert!(!t.contains("](cast/README.md)"), "must not link the 404 README path");
            }
            _ => panic!("expected a generated home page"),
        }
        // a real README.md is used verbatim instead
        fs::write(base.join("README.md"), "# Welcome\n").unwrap();
        match home_page(&base, "proj", &sections).body {
            PageBody::Copy(p) => assert_eq!(p, base.join("README.md")),
            _ => panic!("expected README.md to be used as home"),
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn served_link_maps_readme_landings_to_index() {
        // host#15: mdBook serves a README.md page at index.html but rewrites an in-content
        // README.md link to a dead README.html. A README landing links the served index.
        assert_eq!(served_link("cast/README.md"), "cast/index.md");
        assert_eq!(served_link("README.md"), "index.md");
        // non-README destinations are unchanged.
        assert_eq!(served_link("PLAN.md"), "PLAN.md");
        assert_eq!(served_link("reference/CLAUDE.md"), "reference/CLAUDE.md");
    }

    #[test]
    fn generated_nav_titles_carry_no_em_dash() {
        // host#15: the prose-hygiene rule forbids decoration em-dashes; the generated nav
        // separators use a colon, so the rendered sidebar obeys the rule.
        let base = std::env::temp_dir().join(format!("hl-navtitle-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        for sec in plan_book(&base) {
            assert!(!sec.title.contains('—'), "nav part-title carries an em-dash: {}", sec.title);
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn stamp_field_reads_quoted_values() {
        let t = "template = \"u\"\nrevision = \"r1\"\nname = \"proj\"\n";
        assert_eq!(stamp_field(t, "name").as_deref(), Some("proj"));
        assert_eq!(stamp_field(t, "revision").as_deref(), Some("r1"));
        assert_eq!(stamp_field(t, "missing"), None);
        assert_eq!(stamp_field("name = \"\"\n", "name"), None); // empty = absent
    }

    #[test]
    fn stamp_field_is_comment_and_boundary_robust() {
        // inline comment after a quoted value (the design's own example stamp)
        assert_eq!(stamp_field("revision = \"abc123\"  # adopted rev\n", "revision").as_deref(), Some("abc123"));
        // unquoted value, no spaces around '='
        assert_eq!(stamp_field("baseline=ae1e688\n", "baseline").as_deref(), Some("ae1e688"));
        // unquoted with trailing comment
        assert_eq!(stamp_field("baseline = 7de7cb1 # newest\n", "baseline").as_deref(), Some("7de7cb1"));
        // boundary: `revision` must not match `revisionx`, and a non-match must not
        // abort the search for a later real match
        assert_eq!(stamp_field("revisionx = \"x\"\nrevision = \"r\"\n", "revision").as_deref(), Some("r"));
    }

    #[test]
    fn applied_ids_reads_multiple_provenance_lines() {
        let t = "revision = \"r\"\n\
                 applied = 7de7cb1 recorded=2026-06-18 via=verify\n\
                 applied = ae1e688 recorded=2026-06-18 via=call/0042\n";
        assert_eq!(applied_ids(t), vec!["7de7cb1".to_string(), "ae1e688".to_string()]);
        assert_eq!(baseline_field(t), None);
    }

    #[test]
    fn entry_applied_by_position_or_membership_no_git() {
        let ledger: Vec<String> = ["b6232a5", "c771d60", "b8c54fc", "821a216", "ae1e688", "7de7cb1"]
            .iter().map(|s| s.to_string()).collect();
        let applied = vec!["7de7cb1".to_string()]; // cherry-applied the newest, out of order
        // at/before baseline = applied
        assert!(entry_applied("b6232a5", &ledger, Some("c771d60"), &applied));
        assert!(entry_applied("c771d60", &ledger, Some("c771d60"), &applied));
        // after baseline and not in the set = PENDING (fail-safe: owed work re-lists)
        assert!(!entry_applied("b8c54fc", &ledger, Some("c771d60"), &applied));
        // explicitly in the applied set = applied, even though it is far past baseline
        assert!(entry_applied("7de7cb1", &ledger, Some("c771d60"), &applied));
        // no baseline → only membership counts
        assert!(!entry_applied("b6232a5", &ledger, None, &applied));
        // an orphaned/unknown id (not in ledger) never panics, never silently applied
        assert!(!entry_applied("8c28e33", &ledger, Some("c771d60"), &applied));
    }

    #[test]
    fn stamp_writers_preserve_all_fields() {
        let t = "template = \"u\"\nrevision = \"r\"\nadopted  = \"2026-03-01\"\nname     = \"yarn-agentic\"\n";
        // insert a new field — name and the rest survive (the stamp_body drop bug)
        let t2 = set_stamp_field(t, "baseline", "ae1e688");
        assert_eq!(stamp_field(&t2, "baseline").as_deref(), Some("ae1e688"));
        assert!(t2.contains("name     = \"yarn-agentic\""));
        assert!(t2.ends_with('\n'));
        // update an existing field — name still not dropped
        let t3 = set_stamp_field(&t2, "revision", "rr");
        assert_eq!(stamp_field(&t3, "revision").as_deref(), Some("rr"));
        assert!(t3.contains("name     = \"yarn-agentic\""));
        // append an applied provenance line (append-only)
        let t4 = append_stamp_line(&t3, "applied = 7de7cb1 recorded=2026-06-18 via=verify");
        assert_eq!(applied_ids(&t4), vec!["7de7cb1".to_string()]);
        assert!(t4.contains("name     = \"yarn-agentic\""));
        // round-trip stable: applying the same set is byte-idempotent
        assert_eq!(set_stamp_field(&t3, "revision", "rr"), t3);
    }

    #[test]
    fn validate_ledger_flags_dependency_problems() {
        let mk = |rev: &str, indep: bool, deps: &[&str]| Upgrade {
            revision: rev.into(), title: String::new(), action: String::new(), requires: String::new(),
            independent: indep, depends: deps.iter().map(|s| s.to_string()).collect(), verify: String::new(),
            restates: Vec::new(),
        };
        // clean: A independent, B depends on A
        assert!(validate_ledger(&[mk("A", true, &[]), mk("B", false, &["A"])]).is_empty());
        // self-dependency
        assert!(validate_ledger(&[mk("A", false, &["A"])]).iter().any(|p| p.contains("itself")));
        // both independent and depends
        assert!(validate_ledger(&[mk("A", true, &["B"]), mk("B", false, &[])]).iter().any(|p| p.contains("both")));
        // dependency on an unknown entry
        assert!(validate_ledger(&[mk("A", false, &["Z"])]).iter().any(|p| p.contains("unknown")));
        // cycle A -> B -> A
        assert!(validate_ledger(&[mk("A", false, &["B"]), mk("B", false, &["A"])]).iter().any(|p| p.contains("cycle")));
    }

    // The migration's git logic: map a legacy `revision` to the newest ledger entry
    // that is an ancestor-or-equal of it (the only place git ancestry is used).
    #[cfg(unix)]
    #[test]
    fn derive_baseline_picks_newest_ancestor() {
        let base = std::env::temp_dir().join(format!("hl-baseline-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let g = |args: &[&str]| assert!(git_ok(&base, args), "git {args:?}");
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        let commit = |n: &str| {
            fs::write(base.join(n), n).unwrap();
            assert!(git_ok(&base, &["add", "-A"]));
            assert!(git_ok(&base, &["commit", "-qm", n]));
            git_out(&base, &["rev-parse", "HEAD"]).unwrap()
        };
        let (c1, c2, c3, c4) = (commit("a"), commit("b"), commit("c"), commit("d"));
        let ledger = vec![c1, c2, c3.clone(), c4.clone()];
        // newest ancestor-or-equal of c3 is c3; of c4 (HEAD) is c4
        assert_eq!(derive_baseline(&base, &ledger, &c3).as_deref(), Some(c3.as_str()));
        assert_eq!(derive_baseline(&base, &ledger, &c4).as_deref(), Some(c4.as_str()));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_ledger_id_takes_ordinal_exact_or_unique_prefix() {
        let ids: Vec<String> = ["b6232a5", "c771d60", "7de7cb1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(resolve_ledger_id("2", &ids).unwrap(), "c771d60"); // 1-based ordinal
        assert_eq!(resolve_ledger_id("7de7cb1", &ids).unwrap(), "7de7cb1"); // exact
        assert_eq!(resolve_ledger_id("7de7", &ids).unwrap(), "7de7cb1"); // unique prefix
        assert!(resolve_ledger_id("0", &ids).is_err()); // ordinals are 1-based
        assert!(resolve_ledger_id("9", &ids).is_err()); // out of range
        assert!(resolve_ledger_id("zzzz", &ids).is_err()); // unknown
        let ambig: Vec<String> = ["abcd111", "abcd222"].iter().map(|s| s.to_string()).collect();
        assert!(resolve_ledger_id("abcd", &ambig).is_err()); // ambiguous prefix → refuse, not guess
    }

    #[test]
    fn remove_applied_lines_drops_only_absorbed_ids() {
        let t = "template = \"x\"\nbaseline = \"b\"\n\
                 applied = d3dc5ed recorded=2026-06-19 via=call/0042\n\
                 applied = 7de7cb1 recorded=2026-06-19 via=verify\n\
                 name = \"demo\"\n";
        let out = remove_applied_lines(t, &["d3dc5ed".to_string()]);
        assert!(!out.contains("d3dc5ed")); // absorbed → dropped
        assert!(out.contains("7de7cb1")); // not absorbed → kept
        assert!(out.contains("name = \"demo\"")); // unrelated lines preserved
        assert!(out.contains("baseline = \"b\""));
    }

    #[test]
    fn where_stub_renders_recipe_without_walking_worktrees() {
        let recipe = vec![Software {
            name: "ik".to_string(),
            url: "https://example.test/ik.git".to_string(),
            pin: "b217881".to_string(),
            branch: "main".to_string(),
            worktrees: Vec::new(),
            lines: vec![Worktree {
                branch: "perf/256k".to_string(),
                pin: "a0506f2deadbeef".to_string(),
                store: None,
                host: None,
            }],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, deps_bundle: None, builds: vec![],
        }];
        let s = where_stub(&recipe);
        assert!(s.contains("## ik"));
        assert!(s.contains("b217881"));
        assert!(s.contains("host-lifecycle software --materialize ."));
        assert!(s.contains("perf/256k @ a0506f2deadb"));
    }

    #[test]
    fn spec_page_fences_grow_past_body_backticks() {
        let plain = spec_page("x.allium", "REQUIRE foo\n", "allium");
        assert!(plain.starts_with("# x.allium\n\n```allium\n"));
        assert!(plain.trim_end().ends_with("```"));
        // a body containing a triple fence forces a longer fence
        let nested = spec_page("y.tla", "a\n```\nb", "tla");
        assert!(nested.contains("````tla\n"), "fence longer than the body's run");
    }

    // End-to-end: a tiny repo plans into six lifecycle-ordered sections, every room
    // covered; remove MEMORY.md and the Memory room fails the coverage predicate.
    #[test]
    fn plan_book_covers_every_room_in_lifecycle_order() {
        let base = std::env::temp_dir().join(format!("hl-book-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let mk = |rel: &str, body: &str| {
            let p = base.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        };
        mk("cast/README.md", "# Cast\n");
        mk("cast/mara.md", "# Mara\n");
        mk("PLAN.md", "# Plan\n");
        mk("plan/0001-foundation/README.md", "# 0001 foundation\n");
        mk("plan/0001-foundation/spec/decode.allium", "REQUIRE decode\n");
        mk("plan/0001-foundation/spec/dflash/multi.tla", "---- MODULE M ----\n====\n");
        mk("call/0000-use-records.md", "# Use records\n");
        mk("CLAUDE.md", "# CLAUDE\n");
        mk("BOOTSTRAP.md", "# Bootstrap\n");
        mk("MEMORY.md", "# Memory\n");
        mk(".host-software", "[software \"demo\"]\nurl = https://x.test/d.git\npin = abc123\nworktrees =\n");

        let sections = plan_book(&base);
        let rooms: Vec<&str> = sections.iter().map(|s| s.room).collect();
        assert_eq!(rooms, vec!["cast", "plan", "software", "call", "reference", "memory"]);
        for s in &sections {
            assert!(s.pages.iter().any(page_has_content), "{} room has a content page", s.room);
        }
        // the spec body is rendered as a page (S3), not just a filename bullet
        let plan = sections.iter().find(|s| s.room == "plan").unwrap();
        assert!(plan.pages.iter().any(|p| p.dest == "plan/0001-foundation/spec/decode.allium.md"));
        // nested spec/<topic>/ renders at the mirrored path (issue #7), not dropped
        assert!(plan.pages.iter().any(|p| p.dest == "plan/0001-foundation/spec/dflash/multi.tla.md"));
        assert!(plan.pages.iter().any(|p| matches!(&p.body, PageBody::Inline(s) if s.contains("(spec/dflash/multi.tla.md)"))), "nested spec listed in index");
        // loose root doc lands under Reference; CLAUDE.md is its landing
        let refr = sections.iter().find(|s| s.room == "reference").unwrap();
        assert!(refr.pages.iter().any(|p| p.dest == "CLAUDE.md" && p.depth == 0));
        assert!(refr.pages.iter().any(|p| p.dest == "BOOTSTRAP.md"));

        // SUMMARY: the home page is a prefix chapter ahead of every room (no room is
        // the landing), then the parts in lifecycle order.
        let home = home_page(&base, "proj", &sections);
        assert_eq!(home.dest, "index.md");
        let summary = summary_text(&home, &sections);
        // labelled with the project name (implicit landing), as a prefix chapter
        let home_at = summary.find("[proj](index.md)").expect("home prefix chapter");
        assert_eq!(home.label, "proj");
        let cast_at = summary.find("# Cast: who").unwrap();
        let call_at = summary.find("# Call: why").unwrap();
        let where_at = summary.find("# Software: where").unwrap();
        assert!(home_at < cast_at, "home leads, not Cast");
        assert!(cast_at < where_at && where_at < call_at, "lifecycle order in SUMMARY");

        // every room here has source, so each is required and must render content
        for s in &sections {
            assert!(s.required && s.pages.iter().any(page_has_content), "{} required + rendered", s.room);
        }

        // remove the Memory source → the room is no longer required (tolerant gate):
        // a legitimately-absent room does not fail --check.
        fs::remove_file(base.join("MEMORY.md")).unwrap();
        let sections = plan_book(&base);
        let mem = sections.iter().find(|s| s.room == "memory").unwrap();
        assert!(!mem.required, "absent Memory source → not gated");
        assert!(!mem.pages.iter().any(page_has_content));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn record_layer_segregated_and_marked() {
        let base = std::env::temp_dir().join(format!("hl-record-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let mk = |rel: &str, body: &str| {
            let p = base.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        };
        mk("PLAN.md", "# Plan\n");
        mk("call/0000-live.md", "# Live\n\n- Status: accepted\n- Scope: demo\n");
        mk("call/0001-old.md", "# Old\n\n- Status: superseded by the spine\n");
        mk("plan/0001-foundation/README.md", "# Foundation\n");

        let sections = plan_book(&base);
        // live decision stays in Call; superseded moves out
        let call = sections.iter().find(|s| s.room == "call").unwrap();
        assert!(call.pages.iter().any(|p| p.dest == "call/0000-live.md"));
        assert!(!call.pages.iter().any(|p| p.dest == "call/0001-old.md"));
        // Archive/Record section carries the superseded decision, banner + suffix
        let arch = sections.iter().find(|s| s.room == "archive").expect("archive section");
        let old = arch.pages.iter().find(|p| p.dest == "call/0001-old.md").unwrap();
        assert!(old.label.ends_with("(superseded)"), "label suffixed");
        assert!(matches!(&old.body, PageBody::Inline(s) if s.starts_with("> **Superseded.")), "banner prepended");
        // a plain milestone README (no retire status) stays in its live room, not archived
        assert!(!arch.pages.iter().any(|p| p.dest == "plan/0001-foundation/README.md"), "live plan README not archived");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn set_lock_version_bumps_only_the_named_crate() {
        let lock = "# auto @generated by Cargo\nversion = 4\n\n\
            [[package]]\nname = \"host-grammar\"\nversion = \"0.2.0\"\n\n\
            [[package]]\nname = \"host-lint\"\nversion = \"0.7.0\"\ndependencies = [\n \"host-grammar\",\n]\n\n\
            [[package]]\nname = \"memchr\"\nversion = \"2.8.2\"\n";
        let out = set_lock_version(lock, "host-lint", "0.8.0").expect("crate present");
        // the named crate's own version is bumped …
        assert!(out.contains("name = \"host-lint\"\nversion = \"0.8.0\""));
        // … and nothing else: not the lock-format header, a dependency, or another crate
        assert!(out.contains("version = 4\n"), "lock-format version untouched");
        assert!(out.contains("name = \"host-grammar\"\nversion = \"0.2.0\""));
        assert!(out.contains("name = \"memchr\"\nversion = \"2.8.2\""));
        // a crate not in the lock yields None (no spurious write)
        assert!(set_lock_version(lock, "absent", "9.9.9").is_none());
    }

    // plan/0032: `deps-bundle = <url> <sha256>` parses into the component, and a stanza
    // without it leaves the field None (no bundle, networked build as before).
    #[test]
    fn parse_software_reads_deps_bundle() {
        let with = parse_software(
            "[software \"host-lint\"]\n url=u\n pin=p\n deps-bundle = https://x/vendor-v1.tar.gz abc123\n",
        );
        assert_eq!(
            with[0].deps_bundle,
            Some(("https://x/vendor-v1.tar.gz".to_string(), "abc123".to_string()))
        );
        let without = parse_software("[software \"host-prove\"]\n url=u\n pin=p\n");
        assert!(without[0].deps_bundle.is_none());
    }

    // issue #6: the value normalizer strips a `"..."` wrapper and rejects a stray quote. The
    // exit-2 path in parse_software is the thin wrapper over this pure `None`.
    #[test]
    fn unquote_recipe_token_units() {
        assert_eq!(unquote_recipe_token("main").as_deref(), Some("main"));
        assert_eq!(unquote_recipe_token("\"main\"").as_deref(), Some("main"));
        assert_eq!(unquote_recipe_token("").as_deref(), Some("")); // empty is a value, not malformed
        assert_eq!(unquote_recipe_token("\"\"").as_deref(), Some(""));
        assert_eq!(unquote_recipe_token("\"main"), None); // unbalanced (opening only)
        assert_eq!(unquote_recipe_token("main\""), None); // stray (closing only, no wrapper)
        assert_eq!(unquote_recipe_token("\""), None); // a lone quote
        assert_eq!(unquote_recipe_token("\"a\"b\""), None); // interior quote
    }

    // issue #6: ordinary ASCII quotes an operator wraps around value lines are stripped, across a
    // single-token field, a whitespace list, and a two-token field — never leaked into the ref,
    // path, or hash.
    #[test]
    fn parse_software_strips_value_quotes() {
        let s = parse_software(
            "[software \"host-lint\"]\n url = \"https://x/y.git\"\n pin = \"abc123\"\n worktrees = \"main\" \"dev\"\n artifact = \"target/bin\" \"deadbeef\"\n",
        );
        assert_eq!(s[0].url, "https://x/y.git");
        assert_eq!(s[0].pin, "abc123");
        assert_eq!(s[0].worktrees, vec!["main".to_string(), "dev".to_string()]);
        assert_eq!(s[0].artifact, Some(("target/bin".to_string(), "deadbeef".to_string())));
        // a bare recipe is unchanged (no over-stripping)
        let bare = parse_software("[software \"host-prove\"]\n url=u\n pin=p\n");
        assert_eq!(bare[0].url, "u");
        assert_eq!(bare[0].pin, "p");
    }

    // plan/0032: merging the vendored-sources snippet keeps the existing config (the
    // reproducibility rustflags) and appends the source block, separated by a blank line.
    #[test]
    fn merge_vendor_config_preserves_existing_rustflags() {
        let existing = "[target.'cfg(target_os = \"linux\")']\nrustflags = [\"-C\", \"link-arg=-Wl,--build-id=none\"]\n";
        let snippet = "[source.crates-io]\nreplace-with = \"vendored-sources\"\n\n[source.vendored-sources]\ndirectory = \"vendor\"\n";
        let merged = merge_vendor_config(existing, snippet);
        assert!(merged.contains("--build-id=none"), "existing rustflags preserved");
        assert!(merged.contains("[source.crates-io]"), "source block appended");
        assert!(merged.contains("[source.vendored-sources]"));
        assert!(merged.ends_with('\n'));
        // an empty existing config yields just the snippet (no leading blank line)
        let fresh = merge_vendor_config("", snippet);
        assert!(fresh.starts_with("[source.crates-io]"));
    }

    // plan/0032 hermeticity gate (DetectDepsBundleDrift): a component pinning a bundle is
    // clean when its committed deps-bundle.lock matches the recorded pin, and a HAZARD when
    // the producer's lock has drifted from `.host-software`.
    #[test]
    fn deps_bundle_drift_is_detected() {
        let base = std::env::temp_dir().join(format!("hl-bundle-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("comp").join("main");
        fs::create_dir_all(&wt).unwrap();
        let mk = |url: &str, sha: &str| Software {
            name: "comp".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None,
            deps_bundle: Some((url.into(), sha.into())), builds: vec![],
        };
        fs::write(wt.join("deps-bundle.lock"), "https://x/v1.tgz abc\n").unwrap();
        assert_eq!(provenance_problems(&base, &mk("https://x/v1.tgz", "abc")), 0, "matching lock is clean");
        assert_eq!(provenance_problems(&base, &mk("https://x/v1.tgz", "DIFFERENT")), 1, "drift is a HAZARD");
        let _ = fs::remove_dir_all(&base);
    }

    // issue #6 (engineered): a missing deps-bundle.lock is a lenient onboarding note when git
    // never tracked it, but a HAZARD when it is tracked and deleted (the pin cross-check is then
    // silently bypassed while the worktree-at-pin check still reads clean).
    #[test]
    fn deps_bundle_lock_deletion_hazards_but_onboarding_is_a_note() {
        let base = std::env::temp_dir().join(format!("hl-bundle-lock-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("comp").join("main");
        fs::create_dir_all(&wt).unwrap();
        let g = |args: &[&str]| {
            process::Command::new("git").arg("-C").arg(&wt).args(args).output().unwrap();
        };
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        let mk = || Software {
            name: "comp".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None,
            deps_bundle: Some(("https://x/v.tgz".into(), "abc".into())), builds: vec![],
        };
        // onboarding: a bundle is declared but the lock was never committed → a note, not a fault.
        assert_eq!(provenance_problems(&base, &mk()), 0, "an uncommitted lock is an onboarding note");
        // commit a matching lock → clean.
        fs::write(wt.join("deps-bundle.lock"), "https://x/v.tgz abc\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "lock"]);
        assert_eq!(provenance_problems(&base, &mk()), 0, "a matching tracked lock is clean");
        // delete the tracked lock from the worktree → the cross-check is bypassed → HAZARD.
        fs::remove_file(wt.join("deps-bundle.lock")).unwrap();
        assert_eq!(provenance_problems(&base, &mk()), 1, "a deleted tracked lock hazards");
        let _ = fs::remove_dir_all(&base);
    }

    // plan/0057: an onboarding component (declares a bundle, no committed lock) is pushed onto
    // the owed list and is not a fault; a matching lock and a drifted lock are never owed (a
    // locked component is done, a drifted one is a HAZARD — neither owes a graduation).
    #[test]
    fn deps_bundle_onboarding_is_owed_not_locked_or_drifted() {
        let base = std::env::temp_dir().join(format!("hl-owed-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("software").join("comp").join("main");
        fs::create_dir_all(&wt).unwrap();
        let mk = || Software {
            name: "comp".into(), url: "u".into(), pin: "p".into(),
            branch: "main".into(), worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None,
            deps_bundle: Some(("https://x/v.tgz".into(), "abc".into())), builds: vec![],
        };
        // onboarding: no lock on disk → owed, not a fault.
        let mut owed = Vec::new();
        assert_eq!(provenance_problems_owed(&base, &mk(), &mut owed), 0, "onboarding is not a fault");
        assert_eq!(owed, vec!["comp".to_string()], "onboarding is owed");
        // a matching lock → clean, not owed.
        fs::write(wt.join("deps-bundle.lock"), "https://x/v.tgz abc\n").unwrap();
        let mut owed = Vec::new();
        assert_eq!(provenance_problems_owed(&base, &mk(), &mut owed), 0, "matching lock is clean");
        assert!(owed.is_empty(), "a locked component is not owed");
        // a drifted lock → HAZARD, still not owed (a fault is not a graduation).
        fs::write(wt.join("deps-bundle.lock"), "https://x/v.tgz DIFFERENT\n").unwrap();
        let mut owed = Vec::new();
        assert_eq!(provenance_problems_owed(&base, &mk(), &mut owed), 1, "drift is a HAZARD");
        assert!(owed.is_empty(), "a drifted component is a fault, not owed");
        let _ = fs::remove_dir_all(&base);
    }

    // issue #22: the deps-bundle staging guard reverts the live worktree on Drop — it removes the
    // staged vendor/ dir and restores (or removes) .cargo/config.toml — so an abnormal exit
    // between staging and the explicit restore leaves only the version bump. `restore` disarms.
    #[test]
    fn staged_deps_guard_reverts_on_drop_and_disarms() {
        let base = std::env::temp_dir().join(format!("hl-depsguard-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".cargo")).unwrap();
        let cfg = base.join(".cargo/config.toml");
        // a config the staging created (none existed before) is removed, and vendor/ is dropped.
        fs::create_dir_all(base.join("vendor")).unwrap();
        fs::write(&cfg, "edited by staging\n").unwrap();
        {
            let _g = StagedDepsGuard { work: base.clone(), cfg_backup: None, armed: true };
        }
        assert!(!base.join("vendor").exists(), "the staged vendor dir is removed on drop");
        assert!(!cfg.exists(), "a config that did not exist before staging is removed");
        // a pre-existing config is restored to its original text on drop.
        fs::create_dir_all(base.join("vendor")).unwrap();
        fs::write(&cfg, "STAGED\n").unwrap();
        {
            let _g = StagedDepsGuard { work: base.clone(), cfg_backup: Some("ORIGINAL\n".into()), armed: true };
        }
        assert_eq!(fs::read_to_string(&cfg).unwrap(), "ORIGINAL\n", "the original config is restored");
        assert!(!base.join("vendor").exists());
        // an explicit restore disarms: a later drop does not clobber a subsequent edit.
        fs::write(&cfg, "ORIGINAL\n").unwrap();
        let mut g = StagedDepsGuard { work: base.clone(), cfg_backup: Some("BACKUP\n".into()), armed: true };
        g.restore();
        assert_eq!(fs::read_to_string(&cfg).unwrap(), "BACKUP\n", "restore reverts immediately");
        fs::write(&cfg, "operator edit\n").unwrap();
        drop(g);
        assert_eq!(fs::read_to_string(&cfg).unwrap(), "operator edit\n", "a disarmed guard does not re-restore");
        let _ = fs::remove_dir_all(&base);
    }
}
