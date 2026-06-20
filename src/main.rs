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

use host_grammar::{format_number, is_valid_name};
use host_lint::{is_ci_file, is_scannable, path_ignored, scan_text_with_allow, Match, Severity};

/// The canonical template a project adopts from; recorded in the stamp.
const TEMPLATE_URL: &str = "https://github.com/connollydavid/host-template";
/// The migration stamp: records which template revision a repo adopted.
const STAMP: &str = ".host";
/// The lifecycle manifest (plan/0025): the single tool-readable journal of the
/// phase order + modality, in the template root, replacing the three prose copies.
/// One `[phase "<name>"]` stanza per phase, same git-config style as `.host-software`.
const MANIFEST: &str = "lifecycle.manifest";
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

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("validate") => validate(args.get(2)),
        Some("next") => next(args.get(2)),
        Some("adopt") => adopt(&args[2..]),
        Some("version") => version(args.get(2)),
        Some("classify") => classify(args.get(2)),
        Some("remap") => remap(&args[2..]),
        Some("software") => software(&args[2..]),
        Some("upgrade") => upgrade(&args[2..]),
        Some("book") => book(&args[2..]),
        Some("obligations") => obligations(&args[2..]),
        Some("manifest") => manifest(&args[2..]),
        _ => {
            eprintln!("usage: host-lifecycle <validate|next|adopt|version|classify|remap|software|upgrade|book|obligations|manifest> ...");
            eprintln!("  validate <dir>                — every NNNN-slug entry is well-formed");
            eprintln!("  next <dir>                    — print the next zero-padded number");
            eprintln!("  adopt <dir> <rev> [--dry-run] — scaffold rooms + write the stamp");
            eprintln!("  version <dir>                 — print the adopted template revision");
            eprintln!("  classify <dir>                — print the migration case (a|b|c); refuse a software repo");
            eprintln!("  remap --check <dir>           — tells left after the .host-remap dictionary applies");
            eprintln!("  remap --apply <dir> [--dry-run] — apply the dictionary (archive-first via a clean git tree)");
            eprintln!("  software --materialize <dir>  — clone the bare store(s) + worktrees from .host-software");
            eprintln!("  software --check <dir>        — verify each canonical worktree is at its recorded pin");
            eprintln!("  software --verify-build <dir> — rebuild from the pin and prove the artifact reproduces");
            eprintln!("  software --install-hooks <dir>— install each component's commit hooks + verified binary");
            eprintln!("  upgrade <dir>                 — list template UPGRADING.md actions newer than the stamp");
            eprintln!("  book <dir> [--dry-run]        — generate docs/ + SUMMARY.md (lifecycle order) for mdBook");
            eprintln!("  book --check <dir>            — fail unless every room renders at least one page");
            eprintln!("  obligations <spec.allium>     — every `allium plan` obligation is dispositioned in <stem>.obligations");
            eprintln!("  manifest --check <path>       — the lifecycle manifest is well-formed (orders unique, requires resolve)");
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
        eprintln!("host-lifecycle next <dir>");
        process::exit(2);
    };
    let max = numbered_entries(Path::new(dir))
        .iter()
        .filter_map(|n| n.split('-').next())
        .filter_map(|num| num.parse::<u32>().ok())
        .max();
    let n = max.map_or(0, |m| m + 1);
    println!("{}", format_number(n));
}

/// `adopt <dir> <revision> [--dry-run]` — scaffold the rooms a host needs and
/// write the `.host` stamp recording the template revision adopted.
/// Idempotent: existing rooms are left untouched. `--dry-run` writes nothing.
fn adopt(args: &[String]) {
    let mut dry = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--dry-run" => dry = true,
            _ => pos.push(a),
        }
    }
    let (Some(dir), Some(revision)) = (pos.first(), pos.get(1)) else {
        eprintln!("host-lifecycle adopt <dir> <revision> [--dry-run]");
        process::exit(2);
    };
    let root = Path::new(dir.as_str());
    if !root.is_dir() {
        eprintln!("host-lifecycle: not a directory: {}", root.display());
        process::exit(2);
    }

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
    print_adopt_checklist(revision);
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

/// Print the post-`adopt` checklist. `adopt` scaffolds rooms and the stamp only;
/// registering the template + verification tools and installing the hooks is manual
/// work with no other prompt, so spell it out. The template submodule is step 1: the
/// `upgrade` phase reads `UPGRADING.md` from it, so an adoption that skips it makes
/// the very next phase fail with no ledger to read.
fn print_adopt_checklist(revision: &str) {
    println!("\nnext steps (adopt scaffolds rooms + the stamp only):");
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
        let applied = applied_ids(&stamp);
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
\x20 2. In the host, run: host-lifecycle adopt <host-dir> <revision>\n\
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
/// methodology. Retired decisions (superseded/deprecated/rejected/proposed) pass.
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
    if rules.is_empty() {
        eprintln!("host-lifecycle: no usable entries in {REMAP}");
        process::exit(2);
    }
    let allow = load_allow(root);
    let ignore = load_ignore(root);
    match mode {
        "check" => remap_check(root, &rules, &allow, &ignore),
        "apply" => remap_apply(root, &rules, &ignore, dry),
        _ => unreachable!(),
    }
}

/// Parse `.host-remap`: `old => new` per line, `#` comments and blanks ignored.
/// Sorted longest-`old`-first so `Phase 5.0` is consumed before `Phase 5`.
fn load_remap(root: &Path) -> Vec<Rule> {
    let text = match fs::read_to_string(root.join(REMAP)) {
        Ok(t) => t,
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

/// Apply the rules across a whole file's text, preserving exact line structure
/// (LF/CRLF and whether the file ends in a newline).
fn apply_text(text: &str, rules: &[Rule]) -> String {
    let mut out = String::with_capacity(text.len());
    for chunk in text.split_inclusive('\n') {
        let (body, nl) = match chunk.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (chunk, ""),
        };
        let (body, cr) = match body.strip_suffix('\r') {
            Some(b) => (b, "\r"),
            None => (body, ""),
        };
        out.push_str(&apply_rules(body, rules));
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
        if p.is_dir() {
            if matches!(name.as_str(), ".git" | "target" | "node_modules" | "vendor") {
                continue;
            }
            let rel = p
                .strip_prefix(root)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"));
            if let Some(rel) = &rel {
                if subs.iter().any(|s| s == rel) {
                    continue;
                }
            }
            collect_files(&p, root, subs, out);
        } else if p.is_file() {
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
    let subs = submodule_paths(root);
    let mut files = Vec::new();
    collect_files(root, root, &subs, &mut files);
    files.sort();

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
        if is_spec_ext(f.extension().and_then(|e| e.to_str()).unwrap_or("")) {
            specs += 1;
        }
        let applied = apply_text(&content, rules);
        if applied != content {
            changed += 1;
        }
        scan_text_with_allow(&applied, &src, allow, &mut remaining);
    }
    for m in &remaining {
        let kind = if m.severity == Severity::Warn { "warning" } else { "tell" };
        println!("{}:{}: {kind}: {} ({})", m.file, m.line, m.text, m.term);
    }
    println!(
        "-- {changed} file(s) would change ({specs} spec file(s) scanned); {} undispositioned tell(s) remain",
        remaining.len()
    );
    if remaining.iter().any(|m| m.severity == Severity::Flag) {
        process::exit(1);
    }
    if remaining.iter().any(|m| m.severity == Severity::Warn) {
        process::exit(3);
    }
}

/// `--apply`: write the substitutions. Refuses unless the git tree is clean, so
/// the prior commit archives the originals verbatim (`CLAUDE.md` §6). `--dry-run`
/// previews without the guard and without writing.
fn remap_apply(root: &Path, rules: &[Rule], ignore: &[String], dry: bool) {
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
        let applied = apply_text(&content, rules);
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
struct Software {
    name: String,
    url: String,
    pin: String,
    /// Bare `worktrees = <dir> ...` form: branch derived from the dir suffix, tree
    /// created at the component `pin`. Kept for back-compat.
    worktrees: Vec<String>,
    /// Explicit `worktree = <dir> <branch> <pin>` form: a parallel line on its own
    /// branch at its own pin, faithfully reproducible by `--materialize` (the bare
    /// form silently put a parallel line at the canonical pin — issue #6).
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
    /// Explicit per-platform builds (issue #1): `[build "<name>" "<platform>"]`
    /// subsections, each a distinct toolchain/artifact of the *same* source `pin`.
    /// When non-empty, these replace the flat `build`/`artifact`/… fields above;
    /// when empty, the flat fields form the single default build (back-compat).
    builds: Vec<PlatformBuild>,
}

/// An explicit parallel worktree: a directory checked out on `branch` at `pin`.
/// `dir` is always the in-structure handle (under the host root). When `store` is
/// set the git worktree physically lives there — possibly off-tree / off-filesystem
/// — and `dir` is materialized as a symlink/junction to it, so an agent editing
/// `dir/…` writes the files under test (issue #2). `host`, when set, gates the line
/// to one OS (`std::env::consts::OS`), mirroring a build's `attest_host`: off-platform
/// the line is skipped by `--materialize`/`--check` rather than reported missing.
struct Worktree {
    dir: String,
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

/// `software --materialize|--check <dir>` — realise the `.host-software` recipe.
/// `--materialize` clones each `<name>.git` bare store and adds the canonical
/// worktree `<name>/` at its `pin` (plus any parallel worktrees), idempotently —
/// it skips what already exists. `--check` verifies each canonical worktree is at
/// its recorded pin: the audit that replaces a submodule gitlink's `git submodule
/// status`.
fn software(args: &[String]) {
    let mut mode: Option<&str> = None;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--materialize" => mode = Some("materialize"),
            "--check" => mode = Some("check"),
            "--verify-build" => mode = Some("verify-build"),
            "--install-hooks" => mode = Some("install-hooks"),
            _ => pos.push(a),
        }
    }
    let Some(dir) = pos.first() else {
        eprintln!("host-lifecycle software <--materialize|--check|--verify-build|--install-hooks> <dir>");
        process::exit(2);
    };
    let Some(mode) = mode else {
        eprintln!("host-lifecycle software needs --materialize, --check, --verify-build, or --install-hooks");
        process::exit(2);
    };
    let root = match fs::canonicalize(Path::new(dir.as_str())) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    let recipe = load_software(&root);
    if recipe.is_empty() {
        eprintln!("host-lifecycle: no [software \"<name>\"] stanzas in {SOFTWARE}");
        process::exit(2);
    }
    match mode {
        "materialize" => software_materialize(&root, &recipe),
        "check" => software_check(&root, &recipe),
        "verify-build" => software_verify_build(&root, &recipe),
        "install-hooks" => software_install_hooks(&root, &recipe),
        _ => unreachable!(),
    }
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
    if failed > 0 {
        process::exit(1);
    }
}

/// The install loop, factored out to return `(installed, failed)` counts instead
/// of exiting — keeps it testable.
fn install_hooks(root: &Path, recipe: &[Software]) -> (usize, usize) {
    let hooks_dir = match git_hooks_dir(root) {
        Some(d) => d,
        None => {
            eprintln!("host-lifecycle: not a git repository: {}", root.display());
            return (0, 1);
        }
    };
    if let Err(e) = fs::create_dir_all(&hooks_dir) {
        eprintln!("host-lifecycle: cannot create {}: {e}", hooks_dir.display());
        return (0, 1);
    }
    let mut installed = 0;
    let mut failed = 0;
    for s in recipe {
        let Some(hooks_rel) = &s.hooks else { continue };
        let worktree = root.join(&s.name);
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
        let dst_bin = hooks_dir.join(bin_name);
        let installs = [
            (script.as_path(), hooks_dir.join("pre-commit")),
            (script.as_path(), hooks_dir.join("commit-msg")),
            (bin.as_path(), dst_bin),
        ];
        let mut ok = true;
        for (src, dst) in installs {
            if let Err(e) = copy_executable(src, &dst) {
                println!("FAIL     {} -> {}: {e}", src.display(), dst.display());
                ok = false;
            }
        }
        if ok {
            println!("OK       {} hooks installed (pre-commit, commit-msg, {}) — {provenance}", s.name,
                bin_name.to_string_lossy());
            installed += 1;
        } else {
            failed += 1;
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

/// `--verify-build`: prove reproducibility (the heavy lane). For each component with a
/// `build` recipe, materialize a clean throwaway worktree at the pin, run the recorded
/// build **inside the recorded `toolchain` container** (host#14 — never the ambient
/// rust), hash the `artifact`, and compare to the recorded sha. A `repro-exempt`
/// component citing a real decision is reported (warn) and its rebuild skipped — the
/// escape clause for not-yet-reproducible migrated software (issue #10).
fn software_verify_build(root: &Path, recipe: &[Software]) {
    let mut bad = 0usize;
    let host = std::env::consts::OS;
    for s in recipe {
        let bare = root.join(format!("{}.git", s.name));
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
                    println!("skip     {tag} reproduces on {ah} (host is {host})");
                    continue;
                }
            }
            if let Some(cite) = b.repro_exempt {
                if cited_decision_exists(root, cite) {
                    println!("warn     {tag} repro-exempt ({cite}) — rebuild comparison skipped");
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
                println!("skip     {tag} no `toolchain` pin — cannot verify in a pinned environment (software --check flags this)");
                continue;
            };
            let Some(runtime) = container_runtime() else {
                println!("skip     {tag} no container runtime (docker/podman) — cannot verify in the recorded toolchain {image}");
                continue;
            };
            if !bare.is_dir() {
                println!("MISSING  {}.git (run --materialize)", s.name);
                bad += 1;
                continue;
            }
            // A per-platform verify worktree, so concurrent platform builds of the
            // same source pin do not collide on one tree.
            let suffix = b.platform.map(|p| format!("-{p}")).unwrap_or_default();
            let work = root.join(format!(".host-verify-{}{suffix}", s.name));
            let _ = fs::remove_dir_all(&work);
            let work_s = work.to_string_lossy().to_string();
            if !git_ok(&bare, &["worktree", "add", "--detach", &work_s, &s.pin]) {
                println!("ERROR    {tag} — cannot create a verify worktree at {}", short(&s.pin));
                bad += 1;
                continue;
            }
            // Run the recorded recipe inside the recorded image. The container is
            // root and writes root-owned output into the mounted worktree, so the
            // recipe chowns /src back to the mount owner before exiting, keeping the
            // worktree removable. An optional HOST_LIFECYCLE_DOCKER_NETWORK (e.g.
            // `host`) covers environments whose default docker bridge lacks DNS.
            let work_abs = fs::canonicalize(&work).unwrap_or_else(|_| work.clone());
            let wrapped =
                format!("{build}; rc=$?; chown -R \"$(stat -c '%u:%g' /src)\" /src 2>/dev/null || true; exit $rc");
            let mut cmd = process::Command::new(runtime);
            cmd.arg("run").arg("--rm");
            if let Ok(net) = env::var("HOST_LIFECYCLE_DOCKER_NETWORK") {
                if !net.is_empty() {
                    cmd.arg("--network").arg(net);
                }
            }
            cmd.arg("-v")
                .arg(format!("{}:/src", work_abs.to_string_lossy()))
                .arg("-w")
                .arg("/src")
                .arg(image)
                .arg("sh")
                .arg("-c")
                .arg(&wrapped);
            let built = cmd.status().map(|st| st.success()).unwrap_or(false);
            if !built {
                println!("ERROR    {tag} — build failed in {runtime} {image}: `{build}`");
                bad += 1;
            } else {
                match sha256_file(&work.join(path)) {
                    Some(h) if &h == sha => println!("ok       {tag} rebuild reproduces {path} @ {} (in {image})", short(sha)),
                    Some(h) => {
                        println!("DRIFT    {tag} rebuild is {} but recorded {} — NOT reproducible", short(&h), short(sha));
                        bad += 1;
                    }
                    None => {
                        println!("ERROR    {tag} — built artifact {path} not found");
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
    println!("-- every non-exempt build reproduces its recorded artifact");
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
                worktrees: Vec::new(),
                lines: Vec::new(),
                build: None,
                toolchain: None,
                deploy: None,
                artifact: None,
                repro_exempt: None,
                hooks: None,
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
                "build" => b.build = Some(val.to_string()),
                "toolchain" => b.toolchain = Some(val.to_string()),
                "deploy" => b.deploy = Some(val.to_string()),
                "artifact" => {
                    let f: Vec<&str> = val.split_whitespace().collect();
                    let [path, sha] = f[..] else {
                        eprintln!("host-lifecycle: {SOFTWARE}:{}: `artifact` needs `<path> <sha256>`", i + 1);
                        process::exit(2);
                    };
                    b.artifact = Some((path.to_string(), sha.to_string()));
                }
                "repro-exempt" => b.repro_exempt = Some(val.to_string()),
                "attest-host" => b.attest_host = Some(val.to_string()),
                _ => {}
            }
            continue;
        }
        match key {
            "url" => cur.url = val.to_string(),
            "pin" => cur.pin = val.to_string(),
            "worktrees" => cur.worktrees = val.split_whitespace().map(String::from).collect(),
            "worktree" => {
                // `worktree = <dir> <branch> <pin> [store=<path>] [host=<os>]` — a
                // parallel line, fully pinned; optional external store + OS gate.
                let f: Vec<&str> = val.split_whitespace().collect();
                if f.len() < 3 {
                    eprintln!(
                        "host-lifecycle: {SOFTWARE}:{}: `worktree` needs `<dir> <branch> <pin> [store=<path>] [host=<os>]`",
                        i + 1
                    );
                    process::exit(2);
                }
                let (dir, branch, pin) = (f[0], f[1], f[2]);
                let mut store = None;
                let mut host = None;
                for tok in &f[3..] {
                    if let Some(v) = tok.strip_prefix("store=") {
                        store = Some(v.to_string());
                    } else if let Some(v) = tok.strip_prefix("host=") {
                        host = Some(v.to_string());
                    } else {
                        eprintln!("host-lifecycle: {SOFTWARE}:{}: unknown `worktree` token `{tok}` (expected store=/host=)", i + 1);
                        process::exit(2);
                    }
                }
                cur.lines.push(Worktree {
                    dir: dir.to_string(),
                    branch: branch.to_string(),
                    pin: pin.to_string(),
                    store,
                    host,
                });
            }
            "build" => cur.build = Some(val.to_string()),
            "toolchain" => cur.toolchain = Some(val.to_string()),
            "deploy" => cur.deploy = Some(val.to_string()),
            "artifact" => {
                // `artifact = <path> <sha256>` — the deployed artifact's expected hash.
                let f: Vec<&str> = val.split_whitespace().collect();
                let [path, sha] = f[..] else {
                    eprintln!("host-lifecycle: {SOFTWARE}:{}: `artifact` needs `<path> <sha256>`", i + 1);
                    process::exit(2);
                };
                cur.artifact = Some((path.to_string(), sha.to_string()));
            }
            "repro-exempt" => cur.repro_exempt = Some(val.to_string()),
            "hooks" => cur.hooks = Some(val.to_string()),
            _ => {}
        }
    }
    for s in &out {
        if s.url.is_empty() || s.pin.is_empty() {
            eprintln!("host-lifecycle: {SOFTWARE}: [software \"{}\"] needs both url and pin", s.name);
            process::exit(2);
        }
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
/// in-tree `root/<dir>`.
fn line_target(root: &Path, w: &Worktree) -> PathBuf {
    match &w.store {
        Some(s) => PathBuf::from(s),
        None => root.join(&w.dir),
    }
}

/// True when a host-gated line should be skipped on the current OS.
fn off_platform(host: &Option<String>) -> bool {
    host.as_deref().is_some_and(|h| h != std::env::consts::OS)
}

/// `--materialize`: clone the bare store and add the worktrees, skipping any that
/// already exist. The bare clone needs its remote-tracking refspec set by hand —
/// `git clone --bare` does not write one — before a `fetch`/`worktree` resolves a
/// remote branch.
fn software_materialize(root: &Path, recipe: &[Software]) {
    let mut made = 0usize;
    for s in recipe {
        let bare_name = format!("{}.git", s.name);
        let bare = root.join(&bare_name);
        let canon = root.join(&s.name);
        if bare.exists() {
            println!("skip     {bare_name} (exists)");
        } else {
            if !git_ok(root, &["clone", "--bare", &s.url, &bare_name]) {
                eprintln!("host-lifecycle: git clone --bare failed for {}", s.name);
                process::exit(2);
            }
            git_ok(&bare, &["config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"]);
            git_ok(&bare, &["fetch", "origin"]);
            println!("clone    {bare_name}");
            made += 1;
        }
        if canon.exists() {
            println!("skip     {}/ (exists)", s.name);
        } else {
            let canon_s = canon.to_string_lossy();
            if !git_ok(&bare, &["worktree", "add", &canon_s, &s.pin]) {
                eprintln!("host-lifecycle: worktree add {}/ @ {} failed", s.name, short(&s.pin));
                process::exit(2);
            }
            git_ok(&canon, &["submodule", "update", "--init", "--recursive"]);
            println!("worktree {}/ @ {}", s.name, short(&s.pin));
            made += 1;
        }
        for wt in &s.worktrees {
            let wtp = root.join(wt);
            if wtp.exists() {
                println!("skip     {wt}/ (exists)");
                continue;
            }
            let branch = wt.strip_prefix(&format!("{}.", s.name)).unwrap_or(wt);
            let wtp_s = wtp.to_string_lossy();
            let ok = if git_ok(&bare, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]) {
                git_ok(&bare, &["worktree", "add", &wtp_s, branch])
            } else {
                git_ok(&bare, &["worktree", "add", "-b", branch, &wtp_s, &s.pin])
            };
            if !ok {
                eprintln!("host-lifecycle: worktree add {wt}/ failed");
                process::exit(2);
            }
            git_ok(&wtp, &["submodule", "update", "--init", "--recursive"]);
            println!("worktree {wt}/ ({branch})");
            made += 1;
        }
        for w in &s.lines {
            if off_platform(&w.host) {
                println!("skip     {}/ (host {}, not {})", w.dir, w.host.as_deref().unwrap_or(""), std::env::consts::OS);
                continue;
            }
            let handle = root.join(&w.dir);
            let target = line_target(root, w);
            let target_s = target.to_string_lossy().to_string();
            // The git worktree itself, at the external store or in-tree. `-B` creates
            // or resets `branch` to the recorded `pin`, so a parallel line lands on its
            // own branch at its own commit — not the canonical pin.
            if target.exists() {
                println!("skip     {target_s} (exists)");
            } else if !git_ok(&bare, &["worktree", "add", "-B", &w.branch, &target_s, &w.pin]) {
                eprintln!("host-lifecycle: worktree add {target_s} @ {} failed", short(&w.pin));
                process::exit(2);
            } else {
                git_ok(&target, &["submodule", "update", "--init", "--recursive"]);
                println!("worktree {target_s} ({} @ {})", w.branch, short(&w.pin));
                made += 1;
            }
            // The in-structure handle: a symlink/junction so an external store still
            // surfaces under the host root (issue #2).
            if w.store.is_some() && fs::symlink_metadata(&handle).is_err() {
                if let Err(e) = make_handle(&handle, &target) {
                    eprintln!("host-lifecycle: handle {}/ -> {target_s} failed: {e}", w.dir);
                    process::exit(2);
                }
                println!("handle   {}/ -> {target_s}", w.dir);
                made += 1;
            }
        }
    }
    println!("-- {made} item(s) materialized");
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
    let applied = applied_ids(&stamp);
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

fn software_check(root: &Path, recipe: &[Software]) {
    let mut bad = 0usize;
    for s in recipe {
        let bare = root.join(format!("{}.git", s.name));
        let canon = root.join(&s.name);
        // Where-room invariant (issue #2): every materialized worktree path must
        // stay under the host root. A path that escapes — absolute or `..`-climbing
        // — is the wrong-tree footgun; the sanctioned off-tree store is a `store=`
        // line, whose `dir` is still an in-tree handle.
        if escapes_root(&s.name) {
            println!("HAZARD   {}/ escapes the host root", s.name);
            bad += 1;
            continue;
        }
        for wt in &s.worktrees {
            if escapes_root(wt) {
                println!("HAZARD   {wt}/ escapes the host root");
                bad += 1;
            }
        }
        if !bare.is_dir() {
            println!("MISSING  {}.git (run --materialize)", s.name);
            bad += 1;
            continue;
        }
        if !canon.is_dir() {
            println!("MISSING  {}/ (run --materialize)", s.name);
            bad += 1;
            continue;
        }
        let want = git_out(&bare, &["rev-parse", &s.pin]);
        let have = git_out(&canon, &["rev-parse", "HEAD"]);
        match (want, have) {
            (Some(w), Some(h)) if w == h => println!("ok       {}/ @ {}", s.name, short(&s.pin)),
            (Some(w), Some(h)) => {
                println!("DRIFT    {}/ at {} but pinned to {}", s.name, short(&h), short(&w));
                bad += 1;
            }
            _ => {
                println!("ERROR    {}/ — cannot resolve HEAD or pin", s.name);
                bad += 1;
            }
        }
        // Explicit parallel worktrees: each at its own branch and pin (issue #6),
        // optionally backed by an external store reached through an in-tree handle
        // (issue #2). The pin/branch check runs against the resolved store.
        for w in &s.lines {
            if off_platform(&w.host) {
                println!("skip     {}/ (host {}, not {})", w.dir, w.host.as_deref().unwrap_or(""), std::env::consts::OS);
                continue;
            }
            if escapes_root(&w.dir) {
                println!("HAZARD   {}/ escapes the host root (use store=<path> with an in-tree handle)", w.dir);
                bad += 1;
                continue;
            }
            let handle = root.join(&w.dir);
            let wt = line_target(root, w);
            // A store-backed line: the in-tree handle must resolve to the store.
            if w.store.is_some() {
                match (fs::canonicalize(&handle), fs::canonicalize(&wt)) {
                    (Ok(h), Ok(t)) if h == t => {}
                    (Ok(h), Ok(t)) => {
                        println!("HAZARD   {}/ resolves to {} not the store {}", w.dir, h.display(), t.display());
                        bad += 1;
                        continue;
                    }
                    _ => {
                        println!("HAZARD   {}/ has no in-structure handle to store {} (run --materialize)", w.dir, wt.to_string_lossy());
                        bad += 1;
                        continue;
                    }
                }
            }
            if !wt.is_dir() {
                println!("MISSING  {}/ (run --materialize)", w.dir);
                bad += 1;
                continue;
            }
            let want = git_out(&bare, &["rev-parse", &w.pin]);
            let have = git_out(&wt, &["rev-parse", "HEAD"]);
            let br = git_out(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
            match (want, have) {
                (Some(want), Some(have)) if want == have => match br {
                    Some(br) if br == w.branch => println!("ok       {}/ ({} @ {})", w.dir, w.branch, short(&w.pin)),
                    Some(br) => {
                        println!("DRIFT    {}/ at {} but on branch {} not {}", w.dir, short(&w.pin), br, w.branch);
                        bad += 1;
                    }
                    None => {
                        println!("ok       {}/ @ {}", w.dir, short(&w.pin));
                    }
                },
                (Some(want), Some(have)) => {
                    println!("DRIFT    {}/ at {} but pinned to {}", w.dir, short(&have), short(&want));
                    bad += 1;
                }
                _ => {
                    println!("ERROR    {}/ — cannot resolve HEAD or pin", w.dir);
                    bad += 1;
                }
            }
        }
        // Reproducible-build provenance: deploy line recorded, exemption cited,
        // deployed artifact attested (issue #10).
        bad += provenance_problems(root, s);
        // Verification lanes are mandatory when a spec of their kind exists: a
        // materialized component carrying a `.allium`/`.tla` must run its lane.
        bad += spec_lane_problems(root, s);
    }
    // Worktree-absence coherence (call/0005): a tracked symlink whose target is not
    // itself tracked here points into a separately-materialized path — a software
    // worktree or a tool submodule — so it dangles wherever that path is not
    // materialized (a fresh clone, CI, a partial submodule init).
    for (link, target) in dangling_symlink_hazards(root) {
        println!("HAZARD   {link} -> {target} (symlink into an un-materialized path; not tracked here)");
        bad += 1;
    }
    // Re-check every recorded upgrade claim against the ledger (plan/0022 step 6).
    bad += upgrade_claim_problems(root);
    // #12: a spec under plan/*/spec/ evades the mandatory lanes — co-locate it with software.
    bad += plan_spec_problems(root);
    if bad > 0 {
        eprintln!("-- {bad} item(s) need attention");
        process::exit(1);
    }
    println!("-- all components at their pinned SHA; no worktree-symlink hazards");
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
fn provenance_problems(root: &Path, s: &Software) -> usize {
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
            if dep == s.name || s.lines.iter().any(|w| w.dir == dep) {
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
            let p = root.join(&s.name).join(path);
            if !p.exists() {
                println!("skip     {tag} artifact {path} not present (not a deploy/build host)");
            } else if sha256_file(&p).as_deref() == Some(sha.as_str()) {
                println!("ok       {tag} artifact {path} @ {} (verified)", short(sha));
            } else {
                println!("note     {tag} artifact {path} is a local build (differs from canonical) — proven by --verify-build");
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
    let worktree = root.join(&s.name);
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
        let manifests = read_obligations_text(&worktree);
        for (token, label, lane_present) in [
            ("kani:", "Kani code-conformance", workflows.contains("cargo kani") || workflows.contains("kani-verifier")),
            ("apalache:", "Apalache symbolic", workflows.contains("apalache-mc")),
            ("tlaps:", "TLAPS proof", workflows.contains("tlapm")),
        ] {
            if manifests.contains(token) {
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

/// Concatenate every `.obligations` manifest in the worktree (for tier-declaration
/// substring checks). Skips `.git`, `target`, `node_modules`, like `find_specs`.
fn read_obligations_text(dir: &Path) -> String {
    let mut text = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name == ".git" || name == "target" || name == "node_modules" {
                    continue;
                }
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("obligations") {
                if let Ok(t) = fs::read_to_string(&p) {
                    text.push_str(&t);
                    text.push('\n');
                }
            }
        }
    }
    text
}

/// Walk a worktree (skipping `.git`, `target`, `node_modules`) and report whether
/// any `.allium` spec, `.tla` spec, and `.obligations` manifest exist.
fn find_specs(dir: &Path) -> (bool, bool, bool) {
    let mut allium = false;
    let mut tla = false;
    let mut obligations = false;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name == ".git" || name == "target" || name == "node_modules" {
                    continue;
                }
                stack.push(p);
            } else {
                match p.extension().and_then(|x| x.to_str()) {
                    Some("allium") => allium = true,
                    Some("tla") => tla = true,
                    Some("obligations") => obligations = true,
                    _ => {}
                }
            }
        }
        if allium && tla && obligations {
            break;
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
fn obligations(args: &[String]) {
    let mut pos: Vec<&String> = Vec::new();
    let mut manifest_arg: Option<&String> = None;
    let mut tests_arg: Option<&String> = None;
    let mut prove_arg: Option<&String> = None;
    let mut rederive_arg: Option<&String> = None;
    let mut record_digests_flag = false;
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
            _ => pos.push(&args[i]),
        }
        i += 1;
    }
    let Some(spec) = pos.first() else {
        eprintln!("host-lifecycle obligations <spec.allium> [--manifest <file>] [--tests <dir>] [--prove <dir>] [--rederive <dir>] [--record-digests]");
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
    let plan = match process::Command::new("allium").arg("plan").arg(spec_path).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        Ok(o) => {
            eprintln!("host-lifecycle: `allium plan` failed: {}", String::from_utf8_lossy(&o.stderr).trim());
            process::exit(2);
        }
        Err(e) => {
            eprintln!("host-lifecycle: cannot run `allium plan` (is allium-cli installed?): {e}");
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
    for p in &problems {
        println!("{p}");
    }
    if !problems.is_empty() {
        eprintln!("-- {} obligation(s) undispositioned, stale, missing a test, or UNPROVEN ({})", problems.len(), manifest.display());
        process::exit(1);
    }
    let mode = if rederive_arg.is_some() { "dispositioned; rungs re-derived" } else { "dispositioned" };
    println!("-- all {} obligation(s) {mode}", plan_ids.len());
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
                if let (Some(name), Some(src)) = (disp.strip_prefix("test:"), tests) {
                    let name = name.trim();
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
    let mut rung = Rung { tool: tool.into(), name: name.into(), bound: None, spec: None, inputs: Vec::new() };
    for t in toks {
        if let Some(v) = t.strip_prefix("bound=") {
            rung.bound = Some(v.to_string());
        } else if let Some(v) = t.strip_prefix("spec=") {
            rung.spec = Some(v.to_string());
        } else if let Some(v) = t.strip_prefix("inputs=") {
            rung.inputs = v.split(',').filter(|s| !s.is_empty()).map(String::from).collect();
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

/// Concatenate every file under `dir` (recursively), for substring checks.
fn read_dir_recursive(dir: &Path) -> String {
    let mut text = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(t) = fs::read_to_string(&p) {
                text.push_str(&t);
                text.push('\n');
            }
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
    let applied = applied_ids(&stamp);

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
        // Append-only provenance on the current (possibly just-migrated) stamp.
        let cur = fs::read_to_string(root.join(STAMP)).unwrap_or_else(|_| stamp.clone());
        let new = append_stamp_line(&cur, &format!("applied = {} recorded={} via={}", id, today(), via));
        if let Err(e) = write_atomic(&root.join(STAMP), &new) {
            eprintln!("host-lifecycle: cannot write {STAMP}: {e}");
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
        let cur_stamp = fs::read_to_string(root.join(STAMP)).unwrap_or_else(|_| stamp.clone());
        let s = set_stamp_field(&cur_stamp, "baseline", &new_baseline);
        let s = remove_applied_lines(&s, &absorbed);
        if let Err(e) = write_atomic(&root.join(STAMP), &s) {
            eprintln!("host-lifecycle: cannot write {STAMP}: {e}");
            process::exit(2);
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

/// Root-level `.md` files the book places in a specific room (so the catch-all
/// Reference section does not list them twice).
const PLACED_ROOT_MD: [&str; 7] = ["SUMMARY.md", "README.md", "MEMORY.md", "CLAUDE.md", "PLAN.md", "home.md", "index.md"];

/// A published section of the book — one per room, emitted in lifecycle order
/// (Who → What/When → Where → Why → How → Memory). A section with no content page
/// fails `book --check` (the stub-coverage gate).
struct Section {
    /// The SUMMARY part-title, e.g. "Cast — who".
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

/// One rendered page: where it lands under `docs/`, its sidebar label and indent,
/// and how to produce it.
struct Page {
    /// Path under `docs/`, e.g. `cast/mara.md`.
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

/// `book <dir> [--dry-run]` — generate `book.toml` + `docs/` (SUMMARY in lifecycle
/// order, specs rendered, a Where stub from `.host-software`). `book --check <dir>`
/// fails unless every room renders at least one page with content. The methodology
/// mandates five rooms and two spec formats but shipped no canonical way to publish
/// them; this is that one maintained publisher, so adopters do not hand-roll a
/// generator that drops rooms or re-derives the `call/0005` src-scoping wrong.
fn book(args: &[String]) {
    let mut check = false;
    let mut dry = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "--dry-run" => dry = true,
            _ => pos.push(a),
        }
    }
    let Some(dir) = pos.first() else {
        eprintln!("host-lifecycle book <dir> [--check] [--dry-run]");
        process::exit(2);
    };
    let root = match fs::canonicalize(Path::new(dir.as_str())) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
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
        flat_room(root, "cast", "Cast — who", "cast"),
        plan_plan(root),
        plan_software(root),
        flat_room(root, "call", "Call — why", "call"),
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
            body: PageBody::Inline("# Plan — what & when\n\nMilestones in this project.\n".to_string()),
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
    Section { title: "Plan — what & when".to_string(), room: "plan", required: true, pages }
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
    Section { title: "Software — where".to_string(), room: "software", required: !pages.is_empty(), pages }
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
    Section { title: "Reference — how".to_string(), room: "reference", required: !pages.is_empty(), pages }
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

/// Generate `book.toml` and `docs/` from the plan. `docs/` is rebuilt from scratch
/// (it is generated output, gitignored), so a removed source never lingers.
fn write_book(root: &Path, home: &Page, sections: &[Section], dry: bool) {
    let docs = root.join("docs");
    let all = std::iter::once(home).chain(sections.iter().flat_map(|s| s.pages.iter()));
    if dry {
        println!("write  book.toml (dry-run)");
        println!("write  docs/SUMMARY.md (dry-run)");
        for p in all {
            println!("write  docs/{} (dry-run)", p.dest);
        }
        return;
    }
    let _ = fs::remove_dir_all(&docs);
    if let Err(e) = fs::create_dir_all(&docs) {
        eprintln!("host-lifecycle: cannot create {}: {e}", docs.display());
        process::exit(2);
    }
    if let Err(e) = fs::write(root.join("book.toml"), book_toml(root)) {
        eprintln!("host-lifecycle: cannot write book.toml: {e}");
        process::exit(2);
    }
    let mut count = 0usize;
    for p in all {
        let dest = docs.join(&p.dest);
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
    if let Err(e) = fs::write(docs.join("SUMMARY.md"), summary_text(home, sections)) {
        eprintln!("host-lifecycle: cannot write docs/SUMMARY.md: {e}");
        process::exit(2);
    }
    println!("-- wrote book.toml + {count} page(s) + docs/SUMMARY.md");
}

/// The mdBook config: `src = "docs"` (never `"."`, which would walk the
/// un-materialized worktrees — `call/0005`), the house light/navy theme, and
/// `custom.css` only if the repo ships one. The title is the stamp's `name` (so it
/// is deterministic regardless of the checkout directory), falling back to the
/// directory name when the stamp carries none.
fn book_toml(root: &Path) -> String {
    let title = stamp_title(root);
    let mut s = format!(
        "[book]\nlanguage = \"en\"\nsrc = \"docs\"\ntitle = \"{title}\"\n\n[output.html]\ndefault-theme = \"light\"\npreferred-dark-theme = \"navy\"\n"
    );
    if root.join("custom.css").is_file() {
        s.push_str("additional-css = [\"custom.css\"]\n");
    }
    s
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
            s.push_str(&format!("- [{}]({})\n", sec.title, p.dest));
        }
    }
    Page { dest: "index.md".to_string(), label: name.to_string(), depth: 0, body: PageBody::Inline(s) }
}

/// Render `docs/SUMMARY.md`: the home page as a prefix chapter (mdBook's landing),
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
        "# Software — where\n\nThe action this project produces. Each component is a bare object store \
with worktrees — not committed into this repo; the recipe below is the reproducibility \
anchor. Materialize the worktrees locally with:\n\n```\nhost-lifecycle software --materialize .\n```\n\n",
    );
    for c in recipe {
        s.push_str(&format!("## {}\n\n- url: {}\n- pin: `{}`\n", c.name, c.url, c.pin));
        let mut wts: Vec<String> = c.worktrees.clone();
        for w in &c.lines {
            wts.push(format!("{} ({} @ {})", w.dir, w.branch, short(&w.pin)));
        }
        if wts.is_empty() {
            s.push_str("- worktrees: — (single canonical line)\n");
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

/// Structural validation of a manifest file: each phase carries `order`, `command`
/// and `skill`; `order`s are unique; every `requires` names a real phase that sits
/// earlier (no forward or self dependency). One `ok`/`HAZARD` line per phase; exits
/// non-zero on any HAZARD (so a CI lane can gate the template's own manifest).
fn manifest_check(path: Option<&Path>) {
    let Some(path) = path else {
        eprintln!("usage: host-lifecycle manifest --check <path>");
        process::exit(2);
    };
    let phases = parse_manifest(&read_manifest_or_exit(path));
    if phases.is_empty() {
        eprintln!("HAZARD   {} has no [phase \"...\"] stanzas", path.display());
        process::exit(1);
    }
    let names: Vec<&str> = phases.iter().map(|p| p.name.as_str()).collect();
    let order_of = |n: &str| phases.iter().find(|p| p.name == n).map(|p| p.order);
    let mut hazards = 0;
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
        assert_eq!(apply_text("a Phase 4\nb\n", &r), "a command-execution\nb\n");
        assert_eq!(apply_text("Phase 4", &r), "command-execution");
        assert_eq!(apply_text("x\r\nPhase 4\r\n", &r), "x\r\ncommand-execution\r\n");
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_round_trips() {
        let body = stamp_body("abc123", "2026-06-14");
        assert_eq!(parse_revision(&body).as_deref(), Some("abc123"));
        assert!(body.contains(TEMPLATE_URL));
        assert!(body.contains("2026-06-14"));
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
}

#[cfg(test)]
mod software_tests {
    use super::*;

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
        fs::create_dir_all(base.join("ik/build/bin")).unwrap();
        fs::create_dir_all(base.join("call")).unwrap();
        fs::write(base.join("ik/build/bin/srv"), "BINARY").unwrap();
        fs::write(base.join("call/0009-exempt.md"), "# x\n- Status: accepted\n- Scope: ik\n").unwrap();
        let sha = sha256_file(&base.join("ik/build/bin/srv")).unwrap();

        // A `toolchain` pin is present (host#14): the artifact-hash checks below test
        // the local-build *note* path, not the no-toolchain HAZARD path (tested after).
        let mk = |deploy: &str, art_sha: &str, exempt: Option<&str>| Software {
            name: "ik".into(), url: "u".into(), pin: "p".into(),
            worktrees: vec![], lines: vec![],
            build: None, toolchain: Some("gcc-13".into()),
            deploy: Some(deploy.into()),
            artifact: Some(("build/bin/srv".into(), art_sha.into())),
            repro_exempt: exempt.map(String::from), hooks: None, builds: vec![],
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
            worktrees: vec![], lines: vec![],
            build: None, toolchain: None,
            deploy: Some("ik".into()),
            artifact: Some(("build/bin/srv".into(), sha.clone())),
            repro_exempt: exempt.map(String::from), hooks: None, builds: vec![],
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
            worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None,
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

    // A materialized component carrying a spec must have its CI lane: a `.allium`
    // without `allium check`+`analyse`, or a `.tla` without TLC, is a HAZARD.
    #[test]
    fn spec_lane_gate_requires_a_lane_when_a_spec_is_present() {
        let base = std::env::temp_dir().join(format!("hl-lane-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("comp");
        fs::create_dir_all(wt.join(".github/workflows")).unwrap();
        fs::write(wt.join("thing.allium"), "-- allium: 3\n").unwrap();
        let mk = || Software {
            name: "comp".into(), url: "u".into(), pin: "p".into(),
            worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None, builds: vec![],
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
        let wt = base.join("comp");
        fs::create_dir_all(wt.join(".github/workflows")).unwrap();
        let mk = || Software {
            name: "comp".into(), url: "u".into(), pin: "p".into(),
            worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: None, artifact: None,
            repro_exempt: None, hooks: None, builds: vec![],
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
        let wt = base.join("hl");
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
            worktrees: vec![], lines: vec![],
            build: None, toolchain: None, deploy: Some("host-lint".into()),
            artifact: Some(("target/release/host-lint".into(), art.into())),
            repro_exempt: None, hooks: Some("pre-commit".into()), builds: vec![],
        };
        // worktree at pin + binary present → installs all three files
        assert_eq!(install_hooks(&base, &[mk(&pin, "deadbeef")]), (1, 0));
        let hooks = base.join(".git/hooks");
        assert!(hooks.join("pre-commit").is_file());
        assert!(hooks.join("commit-msg").is_file());
        assert!(hooks.join("host-lint").is_file());
        // worktree off its pin → blocked
        assert_eq!(install_hooks(&base, &[mk("0000000000000000000000000000000000000000", "x")]), (0, 1));

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
\tworktree  = ik.256k perf/256k-single-context a0506f2
";
        let s = parse_software(text);
        assert_eq!(s.len(), 1);
        assert!(s[0].worktrees.is_empty());
        assert_eq!(s[0].lines.len(), 1);
        assert_eq!(s[0].lines[0].dir, "ik.256k");
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
            worktrees: Vec::new(),
            lines: vec![Worktree {
                dir: "demo.line".to_string(),
                branch: "feature".to_string(),
                pin: line_pin.clone(),
                store: None,
                host: None,
            }],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, builds: vec![],
        }];
        software_materialize(&host, &recipe);

        let line = host.join("demo.line");
        assert!(line.is_dir(), "parallel worktree created");
        // at its OWN pin, not the canonical one
        assert_eq!(git_out(&line, &["rev-parse", "HEAD"]).unwrap(), line_pin);
        assert_ne!(line_pin, canon, "fixture sanity: the two pins differ");
        assert_eq!(git_out(&line, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap(), "feature");
        software_check(&host, &recipe); // passes on a matching branch+pin

        let _ = fs::remove_dir_all(&base);
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
    fn worktree_line_parses_store_and_host() {
        let s = parse_software(
            "[software \"x\"]\n  url = u\n  pin = p\n  \
             worktree = ik.win windows/msvc abc123 store=/mnt/d/dev/ik host=linux\n",
        );
        let w = &s[0].lines[0];
        assert_eq!(w.dir, "ik.win");
        assert_eq!(w.branch, "windows/msvc");
        assert_eq!(w.pin, "abc123");
        assert_eq!(w.store.as_deref(), Some("/mnt/d/dev/ik"));
        assert_eq!(w.host.as_deref(), Some("linux"));
        // back-compat: a bare 3-token line carries no store/host
        let b = parse_software("[software \"x\"]\n url=u\n pin=p\n worktree = d br pin\n");
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
            worktrees: Vec::new(),
            lines: vec![Worktree {
                dir: "demo.win".to_string(),
                branch: "win".to_string(),
                pin: line_pin.clone(),
                store: Some(store.to_string_lossy().to_string()),
                host: None,
            }],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, builds: vec![],
        }];
        software_materialize(&host, &recipe);

        let handle = host.join("demo.win");
        // the in-tree handle exists and is a symlink to the external store
        assert!(fs::symlink_metadata(&handle).unwrap().file_type().is_symlink(), "handle is a symlink");
        assert_eq!(fs::canonicalize(&handle).unwrap(), fs::canonicalize(&store).unwrap());
        // the git worktree physically lives at the store, at the line pin
        assert!(store.join(".git").exists(), "worktree at the external store");
        assert_eq!(git_out(&store, &["rev-parse", "HEAD"]).unwrap(), line_pin);
        software_check(&host, &recipe); // passes: handle resolves to store, pin+branch match

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
            worktrees: Vec::new(),
            lines: Vec::new(),
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, builds: vec![],
        }];
        software_materialize(&host, &recipe);

        assert!(host.join("demo.git").is_dir(), "bare store created");
        assert!(host.join("demo").is_dir(), "canonical worktree created");
        assert_eq!(git_out(&host.join("demo"), &["rev-parse", "HEAD"]).unwrap(), pin);
        // check passes (returns without process::exit on a matching pin)
        software_check(&host, &recipe);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn normalize_join_resolves_dotdot() {
        assert_eq!(normalize_join(".claude/skills", "../../host-lint"), "host-lint");
        assert_eq!(normalize_join("a/b", "../c"), "a/c");
        assert_eq!(normalize_join("", "host-lint/SKILL.md"), "host-lint/SKILL.md");
        assert_eq!(normalize_join(".claude/skills", "../../host-lint/SKILL.md"), "host-lint/SKILL.md");
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
    fn book_toml_scopes_src_to_docs() {
        let base = std::env::temp_dir().join(format!("hl-toml-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let toml = book_toml(&base);
        assert!(toml.contains("src = \"docs\""), "never src = \".\" (call/0005)");
        assert!(!toml.contains("src = \".\""));
        assert!(toml.contains("default-theme = \"light\""));
        // no custom.css → no additional-css line
        assert!(!toml.contains("additional-css"));
        fs::write(base.join("custom.css"), "body{}").unwrap();
        assert!(book_toml(&base).contains("additional-css = [\"custom.css\"]"));
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
                assert!(t.contains("](cast/README.md)"), "links the Cast landing");
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
            worktrees: Vec::new(),
            lines: vec![Worktree {
                dir: "ik.256k".to_string(),
                branch: "perf/256k".to_string(),
                pin: "a0506f2deadbeef".to_string(),
                store: None,
                host: None,
            }],
            build: None, toolchain: None, deploy: None, artifact: None, repro_exempt: None, hooks: None, builds: vec![],
        }];
        let s = where_stub(&recipe);
        assert!(s.contains("## ik"));
        assert!(s.contains("b217881"));
        assert!(s.contains("host-lifecycle software --materialize ."));
        assert!(s.contains("ik.256k (perf/256k @ a0506f2deadb)"));
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
        let cast_at = summary.find("# Cast — who").unwrap();
        let call_at = summary.find("# Call — why").unwrap();
        let where_at = summary.find("# Software — where").unwrap();
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
}
