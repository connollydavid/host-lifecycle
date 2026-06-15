//! host-lifecycle — the token-free lifecycle tool for an agentic host.
//!
//! Mechanical, rule-bound work — allocating zero-padded register numbers,
//! validating that names are well-formed, and scaffolding/stamping a repo when
//! it adopts the methodology — kept off the agent's token budget. Names come
//! from `host-grammar`, the same crate `host-lint` checks against, so what this
//! emits is exactly what the checker accepts.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use host_grammar::{format_number, is_valid_name};
use host_lint::{is_ci_file, is_scannable, path_ignored, scan_text_with_allow, Match, Severity};

/// The canonical template a project adopts from; recorded in the stamp.
const TEMPLATE_URL: &str = "https://github.com/connollydavid/host-template";
/// The migration stamp: records which template revision a repo adopted.
const STAMP: &str = ".host";
/// The rooms `adopt` scaffolds (Where = the software submodule, added by hand).
const ROOMS: [&str; 3] = ["cast", "plan", "call"];

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
        Some("upgrade") => upgrade(args.get(2)),
        _ => {
            eprintln!("usage: host-lifecycle <validate|next|adopt|version|classify|remap|software|upgrade> ...");
            eprintln!("  validate <dir>                — every NNNN-slug entry is well-formed");
            eprintln!("  next <dir>                    — print the next zero-padded number");
            eprintln!("  adopt <dir> <rev> [--dry-run] — scaffold rooms + write the stamp");
            eprintln!("  version <dir>                 — print the adopted template revision");
            eprintln!("  classify <dir>                — print the migration case (a|b|c)");
            eprintln!("  remap --check <dir>           — tells left after the .host-remap dictionary applies");
            eprintln!("  remap --apply <dir> [--dry-run] — apply the dictionary (archive-first via a clean git tree)");
            eprintln!("  software --materialize <dir>  — clone the bare store(s) + worktrees from .host-software");
            eprintln!("  software --check <dir>        — verify each canonical worktree is at its recorded pin");
            eprintln!("  upgrade <dir>                 — list template UPGRADING.md actions newer than the stamp");
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
    let mut bad = 0;
    for name in numbered_entries(Path::new(dir)) {
        if !is_valid_name(&name) {
            println!("invalid: {name}");
            bad += 1;
        }
    }
    if bad > 0 {
        eprintln!("{bad} invalid name(s)");
        process::exit(1);
    }
    println!("ok");
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
}

/// `version <dir>` — print the template revision recorded in the stamp.
fn version(dir: Option<&String>) {
    let Some(dir) = dir else {
        eprintln!("host-lifecycle version <dir>");
        process::exit(2);
    };
    match fs::read_to_string(Path::new(dir).join(STAMP))
        .ok()
        .as_deref()
        .and_then(parse_revision)
    {
        Some(rev) => println!("{rev}"),
        None => {
            eprintln!("host-lifecycle: no readable {STAMP} in {dir}");
            process::exit(1);
        }
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
    println!(
        "{}",
        classify_case(root.join(STAMP).is_file(), root.join("CLAUDE.md").is_file())
    );
}

/// The stamp file body — a plain key/value record of the adopted template.
fn stamp_body(revision: &str, date: &str) -> String {
    format!("template = \"{TEMPLATE_URL}\"\nrevision = \"{revision}\"\nadopted  = \"{date}\"\n")
}

/// Pull the `revision` value out of a stamp file's text.
fn parse_revision(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("revision") {
            let v = rest.trim_start().strip_prefix('=')?.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
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

/// A file the remap should touch: scannable by `host-lint`, not a CI file, and not
/// one of our own control files (the dictionary, the allow file, the stamp).
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
    is_scannable(ext)
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
        "-- {changed} file(s) would change; {} undispositioned tell(s) remain",
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
        "-- {changed} file(s) {}",
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
    worktrees: Vec<String>,
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
            _ => pos.push(a),
        }
    }
    let Some(dir) = pos.first() else {
        eprintln!("host-lifecycle software <--materialize|--check> <dir>");
        process::exit(2);
    };
    let Some(mode) = mode else {
        eprintln!("host-lifecycle software needs --materialize or --check");
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
        _ => unreachable!(),
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
            });
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
        match key {
            "url" => cur.url = val.to_string(),
            "pin" => cur.pin = val.to_string(),
            "worktrees" => cur.worktrees = val.split_whitespace().map(String::from).collect(),
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
    }
    println!("-- {made} item(s) materialized");
}

/// `--check`: each component's bare store and canonical worktree must exist, and
/// the worktree must sit at the recorded `pin`. Exit 1 if any is missing or drifted.
fn software_check(root: &Path, recipe: &[Software]) {
    let mut bad = 0usize;
    for s in recipe {
        let bare = root.join(format!("{}.git", s.name));
        let canon = root.join(&s.name);
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
    }
    // Worktree-absence coherence (call/0005): a tracked symlink whose target is not
    // itself tracked here points into a separately-materialized path — a software
    // worktree or a tool submodule — so it dangles wherever that path is not
    // materialized (a fresh clone, CI, a partial submodule init).
    for (link, target) in dangling_symlink_hazards(root) {
        println!("HAZARD   {link} -> {target} (symlink into an un-materialized path; not tracked here)");
        bad += 1;
    }
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

/// One entry in the template's `UPGRADING.md`: an action that became required at a
/// given template revision, to be applied when upgrading a repo stamped older.
struct Upgrade {
    revision: String,
    title: String,
    action: String,
    requires: String,
}

/// `upgrade <dir>` — list the template `UPGRADING.md` actions newer than the repo's
/// `.host` stamp. The mechanical half of a case-(c) version upgrade: it
/// answers "since the revision I adopted, what must I do?" by git ancestry, so a
/// doc diff is no longer the only signal for the structural migrations a revision
/// span introduced.
fn upgrade(dir: Option<&String>) {
    let dir = dir.map(String::as_str).unwrap_or(".");
    let root = match fs::canonicalize(Path::new(dir)) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("host-lifecycle: not a directory: {dir}");
            process::exit(2);
        }
    };
    let Some(rev) = fs::read_to_string(root.join(STAMP)).ok().and_then(|s| parse_revision(&s)) else {
        eprintln!("host-lifecycle: no {STAMP} revision — not an adopted repo");
        process::exit(2);
    };
    let Some(template) = find_template_dir(&root) else {
        eprintln!("host-lifecycle: cannot find the template submodule (none carries UPGRADING.md)");
        process::exit(2);
    };
    let text = match fs::read_to_string(template.join("UPGRADING.md")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("host-lifecycle: cannot read UPGRADING.md: {e}");
            process::exit(2);
        }
    };
    let pending: Vec<Upgrade> = parse_upgrading(&text)
        .into_iter()
        .filter(|e| upgrade_applies(&template, &rev, &e.revision))
        .collect();
    if pending.is_empty() {
        println!("up to date — no UPGRADING.md actions newer than {}", short(&rev));
        return;
    }
    println!("from {} — {} action(s) to apply:", short(&rev), pending.len());
    for e in &pending {
        println!("\n[{}] {}", short(&e.revision), e.title);
        println!("  action:   {}", e.action);
        if !e.requires.is_empty() {
            println!("  requires: {}", e.requires);
        }
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
            _ => {}
        }
    }
    out
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

/// Does an action that landed at template revision `landed` apply to a repo stamped
/// at `have`? Yes when `have` is strictly older (an ancestor, and not equal). If the
/// local template cannot resolve `landed` (not fetched to the target yet) the repo
/// is behind it, so the action applies.
fn upgrade_applies(template: &Path, have: &str, landed: &str) -> bool {
    let Some(landed_sha) = git_out(template, &["rev-parse", "--verify", &format!("{landed}^{{commit}}")]) else {
        return true;
    };
    let Some(have_sha) = git_out(template, &["rev-parse", "--verify", &format!("{have}^{{commit}}")]) else {
        return true;
    };
    have_sha != landed_sha && git_ok(template, &["merge-base", "--is-ancestor", have, landed])
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
    fn classify_by_governance() {
        assert_eq!(classify_case(true, true), "c"); // stamp wins
        assert_eq!(classify_case(true, false), "c");
        assert_eq!(classify_case(false, true), "b");
        assert_eq!(classify_case(false, false), "a");
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

    // An action applies only to a repo stamped strictly older than where it landed.
    #[cfg(unix)]
    #[test]
    fn applies_by_strict_ancestry() {
        let base = std::env::temp_dir().join(format!("hl-upgrade-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let g = |args: &[&str]| assert!(git_ok(&base, args), "git {args:?}");
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        fs::write(base.join("a"), "1").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "one"]);
        let r1 = git_out(&base, &["rev-parse", "HEAD"]).unwrap();
        fs::write(base.join("b"), "2").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "two"]);
        let r2 = git_out(&base, &["rev-parse", "HEAD"]).unwrap();

        assert!(upgrade_applies(&base, &r1, &r2), "older repo gets a newer action");
        assert!(!upgrade_applies(&base, &r2, &r1), "newer repo skips an older action");
        assert!(!upgrade_applies(&base, &r2, &r2), "same revision is not pending");
        assert!(upgrade_applies(&base, &r1, "deadbeefdeadbeef"), "unknown landed → behind it");

        let _ = fs::remove_dir_all(&base);
    }
}
