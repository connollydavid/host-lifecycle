//! host-lifecycle — the token-free lifecycle tool for an agentic host.
//!
//! Mechanical, rule-bound work — allocating zero-padded register numbers,
//! validating that names are well-formed, and scaffolding/stamping a repo when
//! it adopts the methodology — kept off the agent's token budget. Names come
//! from `host-grammar`, the same crate `host-lint` checks against, so what this
//! emits is exactly what the checker accepts.

use std::env;
use std::fs;
use std::path::Path;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use host_grammar::{format_number, is_valid_name};

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
        _ => {
            eprintln!("usage: host-lifecycle <validate|next|adopt|version|classify> ...");
            eprintln!("  validate <dir>                — every NNNN-slug entry is well-formed");
            eprintln!("  next <dir>                    — print the next zero-padded number");
            eprintln!("  adopt <dir> <rev> [--dry-run] — scaffold rooms + write the stamp");
            eprintln!("  version <dir>                 — print the adopted template revision");
            eprintln!("  classify <dir>                — print the migration case (a|b|c)");
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
