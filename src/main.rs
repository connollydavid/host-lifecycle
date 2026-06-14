//! host-lifecycle — the token-free lifecycle tool for an agentic host.
//!
//! Mechanical, rule-bound work — allocating zero-padded register numbers and
//! validating that names are well-formed — kept off the agent's token budget.
//! Names come from `host-grammar`, the same crate `host-lint` checks against,
//! so what this emits is exactly what the checker accepts.

use std::env;
use std::fs;
use std::path::Path;
use std::process;

use host_grammar::{format_number, is_valid_name};

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("validate") => validate(args.get(2)),
        Some("next") => next(args.get(2)),
        _ => {
            eprintln!("usage: host-lifecycle <validate|next> <register-dir>");
            eprintln!("  validate <dir>  — every NNNN-slug entry is well-formed");
            eprintln!("  next <dir>      — print the next zero-padded number");
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
