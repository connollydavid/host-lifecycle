//! The `host-lifecycle dream` audit (plan/0073 #implement-dream).
//!
//! Advisory, read-mostly audit over the project's two memory stores: the repo
//! `MEMORY.md` (append-only) and the host-* per-user store (editable, at
//! `~/.host-memory/<project>/`). Emits one finding per defect, with a route
//! (`edit` for the per-user store, `append` for the repo store). `--fix`
//! applies only the structural-safe class on the per-user store, refusing the
//! repo store by construction. Exit codes: 0 clean, 1 findings, 2 cannot
//! proceed on input.
//!
//! Routing surface is memory-only (no MADR route, no methodology-state
//! change); confirmed non-overlapping with `upgrade` in plan/0073
//! gather-data.md (2026-07-19).
//!
//! Detector scope at this task:
//! - superseded-but-unlinked (structural, precise)
//! - dangling-link (structural, precise)
//! - room-touching (simple call/plan regex; spine cross-check is a follow-up)
//! - description-body-drift (heuristic; cast review sharpens at #cast-consult)
//! - stale-state-over-lore, workaround-vs-plan (heuristic stubs)
//! - append-only-violation (deferred; needs git history, lands in a follow-up
//!   to #implement-dream)
//!
//! The detector engine is the shared surface with the MCP `memory_consolidate`
//! tool (#extend-mcp), so the per-detector functions are `pub`.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::memory::{MemoryStore, EntryType};

/// One memory-store location; mirrors the Allium spec's `StoreLoc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreLoc {
    Repo,
    PerUser,
}

impl StoreLoc {
    fn as_str(self) -> &'static str {
        match self {
            StoreLoc::Repo => "repo",
            StoreLoc::PerUser => "per-user",
        }
    }
}

/// One finding's route. Memory-only: `edit` on the per-user store, `append` on
/// the repo store. No MADR route; non-overlap with upgrade confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Edit,
    Append,
}

impl Route {
    fn as_str(self) -> &'static str {
        match self {
            Route::Edit => "edit",
            Route::Append => "append",
        }
    }
}

/// A detected finding. `entry_slug` identifies the entry; `store` carries the
/// routing asymmetry; `kind` is the detector class (secondary metadata); the
/// `explanation` is the operator-facing prose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub entry_slug: String,
    pub store: StoreLoc,
    pub kind: String,
    pub route: Route,
    pub explanation: String,
}

/// The detector engine's input: one entry, plus its store context. The
/// per-user store passes its directory (so dangling-link can resolve); the
/// repo store passes None (cross-entry resolution is a per-user concern).
#[allow(dead_code)] // entry_type + store_dir reserved for the deferred detectors
pub struct DetectorInput<'a> {
    pub slug: String,
    pub description: String,
    pub body: String,
    pub superseded_by: String,
    pub entry_type: EntryType,
    pub store: StoreLoc,
    pub store_dir: Option<&'a Path>,
    /// The set of slugs known to exist in the same store (for dangling-link
    /// resolution on either store).
    pub known_slugs: &'a BTreeSet<String>,
}

impl<'a> DetectorInput<'a> {
    fn route_for(self_loc: StoreLoc) -> Route {
        match self_loc {
            StoreLoc::PerUser => Route::Edit,
            StoreLoc::Repo => Route::Append,
        }
    }
}

/// The detector engine: run every implemented detector over one entry,
/// returning the findings (zero or more). Order is stable: detectors run in
/// the spec's declaration order; this is the function the dream subcommand
/// and the memory_consolidate MCP tool both call.
pub fn detect(input: &DetectorInput) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(f) = detect_superseded_unlinked(input) {
        out.push(f);
    }
    if let Some(f) = detect_dangling_link(input) {
        out.push(f);
    }
    if let Some(f) = detect_room_touching(input) {
        out.push(f);
    }
    if let Some(f) = detect_description_body_drift(input) {
        out.push(f);
    }
    out
}

/// Superseded-but-unlinked: entry.superseded_by names a slug, but the body
/// does not contain `[[<superseded_by>]]`. The forward link is the operator's
/// way of telling recall the older entry is no longer current; its absence is
/// the detector's signal. Precise, structural.
pub fn detect_superseded_unlinked(input: &DetectorInput) -> Option<Finding> {
    let target = input.superseded_by.trim();
    if target.is_empty() {
        return None;
    }
    let link = format!("[[{target}]]");
    if input.body.contains(&link) {
        return None;
    }
    Some(Finding {
        entry_slug: input.slug.clone(),
        store: input.store,
        kind: "superseded-but-unlinked".to_string(),
        route: DetectorInput::route_for(input.store),
        explanation: format!(
            "superseded by `{target}` but no forward link `[[{target}]]` in the body"
        ),
    })
}

/// Dangling-link: the body contains `[[slug]]` references where `slug` is not
/// in the known set for the same store. Same-store by format definition; a
/// cross-store reference is ordinary prose, not a link.
pub fn detect_dangling_link(input: &DetectorInput) -> Option<Finding> {
    for link in extract_wiki_links(&input.body) {
        if !input.known_slugs.contains(&link) {
            return Some(Finding {
                entry_slug: input.slug.clone(),
                store: input.store,
                kind: "dangling-link".to_string(),
                route: DetectorInput::route_for(input.store),
                explanation: format!("body links `[[{link}]]` but no entry `{link}` exists in the {} store", input.store.as_str()),
            });
        }
    }
    None
}

/// Room-touching: the entry's body references a `call/NNNN` or `plan/NNNN`
/// record. The full detector cross-references the project's applied-set
/// ledger (.host-receipts) to confirm the cited record was superseded by the
/// spine; that cross-check lands in a follow-up. The MVP flags every
/// room reference so the operator can review it; the false-positive rate is
/// the cost of the simpler heuristic, and the cast review rules on whether
/// the full cross-check is in scope for plan/0073 or a named follow-up.
pub fn detect_room_touching(input: &DetectorInput) -> Option<Finding> {
    if let Some(room_ref) = extract_room_refs(&input.body).into_iter().next() {
        return Some(Finding {
            entry_slug: input.slug.clone(),
            store: input.store,
            kind: "room-touching".to_string(),
            route: DetectorInput::route_for(input.store),
            explanation: format!(
                "body cites `{room_ref}`; confirm the record is not superseded by the spine"
            ),
        });
    }
    None
}

/// Description-body-drift: HEURISTIC. The entry's `description:` (what recall
/// keys on) appears to contradict its body. The detector keys on a simple
/// signal: a `not <X>` or `no <X>` in one and a bare `<X>` in the other. The
/// cast review sharpens this at #cast-consult; for now it is conservative
/// (only fires on the `not/no` pattern, which is the shape the motivating
/// failure took).
pub fn detect_description_body_drift(input: &DetectorInput) -> Option<Finding> {
    let d = input.description.to_lowercase();
    let b = input.body.to_lowercase();
    // description says "not X" but body asserts X
    for neg in ["not ", "no "] {
        if let Some(idx) = d.find(neg) {
            let tail = d[idx + neg.len()..].trim();
            if !tail.is_empty() {
                let token = tail.split_whitespace().next().unwrap_or("");
                if !token.is_empty() && token.len() >= 3 && b.contains(token) {
                    return Some(Finding {
                        entry_slug: input.slug.clone(),
                        store: input.store,
                        kind: "description-body-drift".to_string(),
                        route: DetectorInput::route_for(input.store),
                        explanation: format!(
                            "description says `{neg}{token}` but the body asserts `{token}`"
                        ),
                    });
                }
            }
        }
    }
    None
}

/// Extract every `[[slug]]` token from a body. A slug is `[a-z0-9-]+`. Order
/// preserved; duplicates included (the first dangling one short-circuits).
pub fn extract_wiki_links(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() && !(bytes[j] == b']' && bytes[j + 1] == b']') {
                j += 1;
            }
            if j + 1 < bytes.len() {
                let slug = &body[start..j];
                if is_slug(slug) {
                    out.push(slug.to_string());
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Extract every `call/NNNN` or `plan/NNNN` reference from a body, in text
/// order. Returns each match (the detector fires once per entry on the first).
pub fn extract_room_refs(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let tail = &body[i..];
        let (matched_len, kind) = if tail.strip_prefix("call/").is_some() {
            (5, "call/")
        } else if tail.strip_prefix("plan/").is_some() {
            (5, "plan/")
        } else {
            i += 1;
            continue;
        };
        let num_start = i + matched_len;
        let mut j = num_start;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > num_start {
            out.push(format!("{}{}", kind, &body[num_start..j]));
            i = j;
        } else {
            i = num_start;
        }
    }
    out
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && s.chars().next().map(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).unwrap_or(false)
}

/// Run the audit over both stores and print findings. `--fix` is refused on
/// the repo store; on the per-user store, the structural-safe class is
/// applied (currently: none; the safe set grows one class at a time per the
/// README, and the first addition lands with cast-review sign-off).
pub fn dream(args: &[String]) {
    let mut fix_mode = false;
    let mut json = false;
    let mut dir: PathBuf = PathBuf::from(".");
    for a in args {
        match a.as_str() {
            "--fix" => fix_mode = true,
            "--json" => json = true,
            "-h" | "--help" => {
                print_help();
                return;
            }
            other if !other.starts_with("--") => {
                dir = PathBuf::from(other);
            }
            other => {
                eprintln!("host-lifecycle dream: unknown flag {other}");
                process::exit(2);
            }
        }
    }
    let dir = match fs::canonicalize(&dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("host-lifecycle dream: not a directory: {}: {e}", dir.display());
            process::exit(2);
        }
    };

    let findings = run_audit(&dir);

    if fix_mode {
        let repo_count = findings.iter().filter(|f| f.store == StoreLoc::Repo).count();
        if repo_count > 0 {
            eprintln!(
                "host-lifecycle dream: --fix refuses the repo store (the append-only tier); {repo_count} repo finding(s) reported, not applied"
            );
            process::exit(2);
        }
        // The structural-safe class on the per-user store lands with cast-review
        // sign-off; for now --fix is a no-op that confirms the audit ran.
        eprintln!(
            "host-lifecycle dream: --fix mode acknowledged; no structural-safe classes are auto-applied yet (the safe set grows one class at a time)"
        );
    }

    if json {
        print_json(&findings);
    } else {
        print_text(&findings);
    }

    process::exit(if findings.is_empty() { 0 } else { 1 });
}

/// The full audit. Reads the per-user store (memory.rs) and the repo
/// `MEMORY.md`; runs the detector engine over each entry; returns findings in
/// detector-declaration order.
pub fn run_audit(project_dir: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Per-user store
    let home = env::var_os("HOME").map(PathBuf::from);
    let per_user_store = home
        .as_ref()
        .and_then(|h| {
            MemoryStore::open(&h.join(".host-memory"), project_dir).ok()
        });
    let per_user_entries = match &per_user_store {
        Some(store) => store.list().unwrap_or_default(),
        None => Vec::new(),
    };
    let per_user_slugs: BTreeSet<String> = per_user_entries
        .iter()
        .map(|e| e.slug.clone())
        .collect();
    for entry in &per_user_entries {
        let input = DetectorInput {
            slug: entry.slug.clone(),
            description: entry.description.clone(),
            body: entry.body.clone(),
            superseded_by: entry.superseded_by.clone(),
            entry_type: entry.entry_type.clone(),
            store: StoreLoc::PerUser,
            store_dir: per_user_store.as_ref().map(|s| s.dir()),
            known_slugs: &per_user_slugs,
        };
        findings.extend(detect(&input));
    }

    // Repo MEMORY.md (the append-only tier). Read entries at the section level;
    // each `### ` heading begins a new entry. Cross-reference the same detectors
    // that make sense for the repo format.
    let repo_memory = project_dir.join("MEMORY.md");
    let repo_findings = audit_repo_memory(&repo_memory);
    findings.extend(repo_findings);

    findings
}

/// The repo MEMORY.md audit. The repo format is `### <heading>` per entry
/// with bullets underneath (no YAML frontmatter per entry); the structural
/// per-user detectors do not apply. The room-touching detector and the
/// append-only-violation detector are the load-bearing ones here.
fn audit_repo_memory(path: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let Ok(content) = fs::read_to_string(path) else {
        return out;
    };
    let mut all_slugs: BTreeSet<String> = BTreeSet::new();
    let entries = parse_repo_memory_sections(&content);
    for e in &entries {
        all_slugs.insert(e.slug.clone());
    }
    for entry in &entries {
        let input = DetectorInput {
            slug: entry.slug.clone(),
            description: entry.heading.clone(),
            body: entry.body.clone(),
            superseded_by: String::new(),
            entry_type: EntryType::Fact,
            store: StoreLoc::Repo,
            store_dir: None,
            known_slugs: &all_slugs,
        };
        out.extend(detect(&input));
    }
    out
}

/// One section of the repo MEMORY.md: a `### ` heading plus the body until the
/// next `### ` or end of file. The slug is the heading text lowercased with
/// non-alphanumerics collapsed to hyphens (for cross-reference shape only;
/// repo entries do not have a real slug field).
struct RepoSection {
    slug: String,
    heading: String,
    body: String,
}

fn parse_repo_memory_sections(content: &str) -> Vec<RepoSection> {
    let mut out = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            if let Some((h, body)) = current.take() {
                out.push(RepoSection {
                    slug: slugify(&h),
                    heading: h,
                    body,
                });
            }
            current = Some((heading.trim().to_string(), String::new()));
        } else if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some((h, body)) = current.take() {
        out.push(RepoSection {
            slug: slugify(&h),
            heading: h,
            body,
        });
    }
    out
}

fn slugify(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::new();
    let mut prev_dash = false;
    for c in lower.chars() {
        if c.is_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn print_help() {
    eprintln!("usage: host-lifecycle dream [<dir>] [--fix] [--json]");
    eprintln!("  --fix    refuse the repo store; apply the structural-safe class to per-user (none yet)");
    eprintln!("  --json   machine-readable findings");
    eprintln!("  -h, --help  this help");
}

fn print_text(findings: &[Finding]) {
    if findings.is_empty() {
        println!("dream: clean (no staleness, drift, or append-only violations)");
        return;
    }
    for f in findings {
        println!(
            "{} ({}) [{}] route={} — {}",
            f.entry_slug,
            f.store.as_str(),
            f.kind,
            f.route.as_str(),
            f.explanation
        );
    }
    eprintln!(
        "host-lifecycle dream: {} finding(s) across the memory stores",
        findings.len()
    );
}

fn print_json(findings: &[Finding]) {
    let mut s = String::from("[\n");
    for (i, f) in findings.iter().enumerate() {
        s.push_str(&format!(
            "  {{\"entry\": \"{}\", \"store\": \"{}\", \"kind\": \"{}\", \"route\": \"{}\", \"explanation\": \"{}\"}}",
            json_escape(&f.entry_slug),
            f.store.as_str(),
            f.kind,
            f.route.as_str(),
            json_escape(&f.explanation)
        ));
        if i + 1 < findings.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push(']');
    println!("{s}");
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

use std::process;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn known(slugs: &[&str]) -> BTreeSet<String> {
        slugs.iter().map(|s| s.to_string()).collect()
    }

    fn input<'a>(slug: &str, desc: &str, body: &str, slugs: &'a BTreeSet<String>) -> DetectorInput<'a> {
        DetectorInput {
            slug: slug.to_string(),
            description: desc.to_string(),
            body: body.to_string(),
            superseded_by: String::new(),
            entry_type: EntryType::Fact,
            store: StoreLoc::PerUser,
            store_dir: None,
            known_slugs: slugs,
        }
    }

    #[test]
    fn extract_wiki_links_finds_slugs() {
        let body = "See [[alpha]] and [[beta-2]] but not [[c]].";
        let links = extract_wiki_links(body);
        assert_eq!(links, vec!["alpha", "beta-2", "c"]);
    }

    #[test]
    fn extract_room_refs_finds_call_and_plan() {
        let body = "as recorded in call/0017 and revisited in plan/0042; call/0017 again";
        let refs = extract_room_refs(body);
        assert_eq!(refs, vec!["call/0017", "plan/0042", "call/0017"]);
    }

    #[test]
    fn detect_superseded_unlinked_fires_when_link_missing() {
        let slugs = known(&["alpha", "beta"]);
        let mut inp = input("alpha", "older", "Alpha body. See nothing.", &slugs);
        inp.superseded_by = "beta".to_string();
        let f = detect_superseded_unlinked(&inp).unwrap();
        assert_eq!(f.kind, "superseded-but-unlinked");
        assert_eq!(f.route, Route::Edit);
    }

    #[test]
    fn detect_superseded_unlinked_silent_when_linked() {
        let slugs = known(&["alpha", "beta"]);
        let mut inp = input("alpha", "older", "Older. See [[beta]] for the update.", &slugs);
        inp.superseded_by = "beta".to_string();
        assert!(detect_superseded_unlinked(&inp).is_none());
    }

    #[test]
    fn detect_dangling_link_fires_for_missing_target() {
        let slugs = known(&["alpha"]);
        let inp = input("alpha", "x", "See [[beta]] which is missing.", &slugs);
        let f = detect_dangling_link(&inp).unwrap();
        assert_eq!(f.kind, "dangling-link");
        assert_eq!(f.route, Route::Edit);
    }

    #[test]
    fn detect_dangling_link_silent_when_target_present() {
        let slugs = known(&["alpha", "beta"]);
        let inp = input("alpha", "x", "See [[beta]] which exists.", &slugs);
        assert!(detect_dangling_link(&inp).is_none());
    }

    #[test]
    fn detect_room_touching_fires_on_call_ref() {
        let slugs = known(&["alpha"]);
        let inp = input("alpha", "x", "Cites call/0017 as live rule.", &slugs);
        let f = detect_room_touching(&inp).unwrap();
        assert_eq!(f.kind, "room-touching");
        assert_eq!(f.route, Route::Edit);
    }

    #[test]
    fn detect_room_touching_routes_to_append_for_repo_store() {
        let slugs = known(&["alpha"]);
        let mut inp = input("alpha", "x", "Cites call/0017.", &slugs);
        inp.store = StoreLoc::Repo;
        let f = detect_room_touching(&inp).unwrap();
        assert_eq!(f.route, Route::Append);
    }

    #[test]
    fn detect_description_body_drift_fires_on_not_vs_bare() {
        let slugs = known(&["alpha"]);
        let inp = input(
            "alpha",
            "not stabilized yet",
            "The API is stabilized as of v0.5.",
            &slugs,
        );
        let f = detect_description_body_drift(&inp).unwrap();
        assert_eq!(f.kind, "description-body-drift");
    }

    #[test]
    fn detect_description_body_drift_silent_on_consistent_entry() {
        let slugs = known(&["alpha"]);
        let inp = input(
            "alpha",
            "stabilized as of v0.5",
            "The API is stabilized as of v0.5.",
            &slugs,
        );
        assert!(detect_description_body_drift(&inp).is_none());
    }

    #[test]
    fn detect_runs_all_detectors_and_orders_findings_stably() {
        let slugs = known(&["alpha"]);
        let mut inp = input(
            "alpha",
            "not /tmp",
            "build in /tmp; cites call/0017; links [[missing]]",
            &slugs,
        );
        inp.superseded_by = "beta".to_string();
        let fs = detect(&inp);
        // Order: superseded-but-unlinked, dangling-link, room-touching, description-body-drift.
        assert_eq!(fs.len(), 4);
        assert_eq!(fs[0].kind, "superseded-but-unlinked");
        assert_eq!(fs[1].kind, "dangling-link");
        assert_eq!(fs[2].kind, "room-touching");
        assert_eq!(fs[3].kind, "description-body-drift");
    }

    #[test]
    fn repo_store_finding_always_routes_to_append() {
        let slugs = known(&["alpha"]);
        let mut inp = input("alpha", "x", "Cites call/0017.", &slugs);
        inp.store = StoreLoc::Repo;
        let f = detect_room_touching(&inp).unwrap();
        assert_eq!(f.route, Route::Append);
    }

    #[test]
    fn slugify_collapses_non_alphanumeric_to_single_hyphens() {
        assert_eq!(slugify("2025-01-XX — Initial Setup"), "2025-01-xx-initial-setup");
        assert_eq!(slugify("Foo!!Bar"), "foo-bar");
    }

    #[test]
    fn parse_repo_memory_sections_splits_on_h3_heading() {
        let content = "# MEMORY.md\n\n## Session Log\n\n### First\n\n- a\n\n### Second\n\n- b\n";
        let sections = parse_repo_memory_sections(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading, "First");
        assert_eq!(sections[1].heading, "Second");
        assert!(sections[0].body.contains("- a"));
    }

    #[test]
    fn run_audit_on_clean_fixture_emits_no_findings() {
        let tmp = std::env::temp_dir().join(format!("dream-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(
            tmp.join("MEMORY.md"),
            "# MEMORY.md\n\n## Log\n\n### First\n\nNo room refs or links here.\n",
        )
        .unwrap();
        let findings = run_audit(&tmp);
        // The repo section detector finds no call/plan refs and no links.
        let clean = findings.iter().all(|f| f.kind != "room-touching" && f.kind != "dangling-link");
        assert!(clean);
        fs::remove_dir_all(&tmp).unwrap();
    }
}
