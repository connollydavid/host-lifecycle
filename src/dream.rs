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

/// A finding's confidence (call/0045): `Confirmed` is mechanically evidenced
/// on the stores and gates the run; `ReviewPrompt` is an unverified hypothesis
/// whose lead remedy is a review note. The label travels in the printed
/// explanation and suggestion, not only here: a 4B reads the prose, not the
/// metadata (the W1 lineage).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    Confirmed,
    ReviewPrompt,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Confirmed => "confirmed",
            Confidence::ReviewPrompt => "review-prompt",
        }
    }
}

/// The per-user tier's declared in-use state, read from the repo-side marker
/// file (call/0045). `Absent` means the tier was never initialized anywhere;
/// `Stamped` declares it in use; `Retired` declares it out of use (an operator
/// act). There is no silent path back from `Retired`: a store observed after
/// retirement is a contradiction finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TierState {
    Absent,
    Stamped,
    Retired,
}

/// The declared applicability table (call/0045): each detector names the
/// stores its format fits, beside the format fact that justifies it. The
/// `detect` engine enforces these rows and the coverage lines are generated
/// from the same rows, so the output and the engine cannot disagree.
pub const APPLICABILITY: &[(&str, &[StoreLoc], &str)] = &[
    ("superseded-but-unlinked", &[StoreLoc::PerUser], "the repo format has no supersession field; supersession there rides a forward link in an appended correction"),
    ("dangling-link", &[StoreLoc::Repo, StoreLoc::PerUser], "the [[slug]] notation spans both tiers; resolution is against the union"),
    ("room-touching", &[StoreLoc::Repo, StoreLoc::PerUser], "room references appear in either tier's prose"),
    ("description-body-drift", &[StoreLoc::PerUser], "the repo format has no per-entry description field"),
    ("stale-state-over-lore", &[StoreLoc::PerUser], "entry types exist only in the per-user frontmatter"),
    ("workaround-vs-plan", &[StoreLoc::PerUser], "entry types exist only in the per-user frontmatter"),
    ("append-only-violation", &[StoreLoc::Repo], "the append-only discipline binds the repo log only"),
];

/// Whether a detector's declared applicability covers a store. Unknown kinds
/// default to applicable so a new detector cannot be silently skipped by a
/// missing row.
pub fn applies(kind: &str, loc: StoreLoc) -> bool {
    APPLICABILITY
        .iter()
        .find(|row| row.0 == kind)
        .map(|row| row.1.contains(&loc))
        .unwrap_or(true)
}

/// Compose the verbatim operator imperative (Wren's W1 finding). The route
/// determines the leading verb and the anti-action tail — the load-bearing
/// two-store asymmetry. The fen-acceptance probe (plan/0073) showed a 4B reads
/// this natural-language imperative and ignores the `route=` token, so the
/// suggestion must carry the routing decision in prose, not just metadata.
/// `goal` is the detector-specific fix, phrased store-neutrally to slot after
/// either frame's "to ...".
fn suggestion_for(route: Route, slug: &str, goal: &str) -> String {
    match route {
        Route::Edit => format!(
            "Edit the per-user entry `{slug}` in place to {goal}. Do not append to the repo log."
        ),
        Route::Append => format!(
            "Append a new dated entry to the repo MEMORY.md to {goal}. Do not edit the existing entry in place."
        ),
    }
}

/// A detected finding. `entry_slug` identifies the entry; `store` carries the
/// routing asymmetry; `kind` is the detector class (secondary metadata); the
/// `explanation` is the operator-facing prose; the `suggestion` is the verbatim
/// route-carrying imperative (W1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub entry_slug: String,
    pub store: StoreLoc,
    pub kind: String,
    pub route: Route,
    pub confidence: Confidence,
    /// The dangling-link absence state ("tier-unused", "uninitialized-here",
    /// "entry-missing"), empty for every other kind. Machine-readable so a new
    /// dangle cannot hide inside a standing baseline (call/0045 condition on
    /// per-state counts).
    pub state: String,
    pub explanation: String,
    pub suggestion: String,
}

/// The detector engine's input: one entry, plus its store context. The
/// per-user store passes its directory (so dangling-link can resolve); the
/// repo store passes None (cross-entry resolution is a per-user concern).
#[allow(dead_code)] // entry_type + store_dir reserved for the deferred detectors
#[derive(Clone)]
pub struct DetectorInput<'a> {
    pub slug: String,
    pub description: String,
    pub body: String,
    pub superseded_by: String,
    pub entry_type: EntryType,
    pub store: StoreLoc,
    pub store_dir: Option<&'a Path>,
    /// The union of slugs known to exist across BOTH stores (call/0045): a
    /// `[[slug]]` link resolves against the whole memory graph, not one tier.
    pub known_slugs: &'a BTreeSet<String>,
    /// The per-user tier's declared state, from the repo-side marker.
    pub tier: TierState,
    /// The tier marker's provenance line (stamp or retirement date and author),
    /// empty when the marker is absent; travels into finding explanations.
    pub tier_provenance: &'a str,
    /// Whether an initialized per-user store is present on this machine.
    pub store_present: bool,
}

impl<'a> DetectorInput<'a> {
    fn route_for(self_loc: StoreLoc) -> Route {
        match self_loc {
            StoreLoc::PerUser => Route::Edit,
            StoreLoc::Repo => Route::Append,
        }
    }
}

/// The detector engine: run every implemented single-entry detector over one
/// entry, returning the findings (zero or more). Order is stable: detectors
/// run in the spec's declaration order; this is the function the dream
/// subcommand and the memory_consolidate MCP tool both call.
///
/// Cross-entry detectors (workaround-vs-plan) and history detectors
/// (append-only-violation) live in `detect_cross` and `audit_repo_memory`
/// respectively; they need wider context than a single DetectorInput carries.
pub fn detect(input: &DetectorInput) -> Vec<Finding> {
    let mut out = Vec::new();
    // The applicability table is the single enforcement point (call/0045):
    // a detector whose declared stores exclude this entry's store is skipped
    // here, and the coverage lines report the same rows.
    if applies("superseded-but-unlinked", input.store) {
        if let Some(f) = detect_superseded_unlinked(input) {
            out.push(f);
        }
    }
    if applies("dangling-link", input.store) {
        if let Some(f) = detect_dangling_link(input) {
            out.push(f);
        }
    }
    if applies("room-touching", input.store) {
        if let Some(f) = detect_room_touching(input) {
            out.push(f);
        }
    }
    if applies("description-body-drift", input.store) {
        if let Some(f) = detect_description_body_drift(input) {
            out.push(f);
        }
    }
    if applies("stale-state-over-lore", input.store) {
        if let Some(f) = detect_stale_state_over_lore(input) {
            out.push(f);
        }
    }
    out
}

/// Run cross-entry detectors over a full store's worth of entries. Each
/// detector receives the entry under review plus the slice of other entries
/// in the same store, so it can spot pair-level patterns
/// (workaround-vs-plan) the single-entry pass cannot.
pub fn detect_cross<'a>(entries: &'a [DetectorInput<'a>]) -> Vec<Finding> {
    let mut out = Vec::new();
    for entry in entries {
        if let Some(f) = detect_workaround_vs_plan(entry, entries) {
            out.push(f);
        }
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
    let route = DetectorInput::route_for(input.store);
    Some(Finding {
        entry_slug: input.slug.clone(),
        store: input.store,
        kind: "superseded-but-unlinked".to_string(),
        confidence: Confidence::Confirmed,
        state: String::new(),
        route,
        explanation: format!(
            "superseded by `{target}` but no forward link `[[{target}]]` in the body"
        ),
        suggestion: suggestion_for(
            route,
            &input.slug,
            &format!("record that it is superseded by `{target}` with a forward `[[{target}]]` link"),
        ),
    })
}

/// Dangling-link: the body contains `[[slug]]` references that resolve in
/// NEITHER store's slug set (the union, call/0045). What an unresolved link
/// means depends on the tier marker and the store's presence on this machine,
/// so the one detector carries three states with distinct confidences and
/// remedies. Every remedy leads with one safe action and keeps the
/// append-only anti-action tail; a link is never dropped on one machine's
/// absence alone.
pub fn detect_dangling_link(input: &DetectorInput) -> Option<Finding> {
    for link in extract_wiki_links(&input.body) {
        if input.known_slugs.contains(&link) {
            continue;
        }
        let route = DetectorInput::route_for(input.store);
        let (confidence, state, explanation, suggestion) = match (input.tier, input.store_present) {
            // Tier declared in use, store absent on this machine: unresolvable
            // here, possibly real elsewhere. Advisory; the lead action is the
            // safe no-op, and initialization stays the operator's act.
            (TierState::Stamped, false) => (
                Confidence::ReviewPrompt,
                "uninitialized-here",
                format!(
                    "review prompt: body links `[[{link}]]`, which resolves in neither store present on this machine (tier marker: stamped {}; per-user store absent here)",
                    input.tier_provenance
                ),
                format!(
                    "Leave the `[[{link}]]` link standing and report it. If this machine should carry the per-user store, the operator initializes or seeds it; the link may resolve on the machine that carries the store. Do not drop the link on this machine's evidence alone; do not edit the entry in place."
                ),
            ),
            // Tier in use (declared, or a store observed ahead of its stamp),
            // store present, target missing: dangling within a live graph.
            (TierState::Stamped, true) | (TierState::Absent, true) => (
                Confidence::ReviewPrompt,
                "entry-missing",
                format!(
                    "review prompt: body links `[[{link}]]`, which resolves in neither the repo log nor the per-user store on this machine"
                ),
                suggestion_for(
                    route,
                    &input.slug,
                    &format!("resolve the broken `[[{link}]]` reference (create the entry `{link}` or correct the link)"),
                ),
            ),
            // Tier declared out of use by retirement: confirmed, through the
            // store-correct frame (a per-user entry can still exist here, the
            // contradiction scenario, and must route edit, never a repo
            // append). The initialization fork does not apply from retired;
            // re-opting in rides the contradiction protocol.
            (TierState::Retired, _) => (
                Confidence::Confirmed,
                "tier-unused",
                format!(
                    "body links `[[{link}]]`, which resolves in neither the repo log nor the per-user tier (tier marker: retired {})",
                    input.tier_provenance
                ),
                {
                    let mut s = suggestion_for(
                        route,
                        &input.slug,
                        &format!("resolve the broken `[[{link}]]` reference (fix the slug, or drop the link with the operator's confirmation)"),
                    );
                    s.push_str(" To re-opt into the per-user tier instead, the operator appends a correction that records the decision, removes the retired marker, and lets the next dream run re-stamp; initialization and stamping are never the auditing agent's act.");
                    s
                },
            ),
            // Tier never initialized anywhere: genuinely dangling, confirmed,
            // with the operator's initialization fork leading the remedy
            // (call/0045 guardrails). Repo-only in practice: with no store on
            // any machine, no per-user entry exists to carry a link.
            (TierState::Absent, false) => (
                Confidence::Confirmed,
                "tier-unused",
                format!(
                    "body links `[[{link}]]`, which resolves in neither the repo log nor the per-user tier (tier marker: absent; the tier was never initialized)"
                ),
                format!(
                    "If a per-user store is intended, the operator initializes it and commits the tier marker; these findings then re-tier advisory. Otherwise fix the slug or drop the forward `[[{link}]]` link, with the operator's confirmation, via a new appended correction to the repo MEMORY.md. Never edit the existing entry in place; initialization and stamping are never the auditing agent's act."
                ),
            ),
        };
        return Some(Finding {
            entry_slug: input.slug.clone(),
            store: input.store,
            kind: "dangling-link".to_string(),
            route,
            confidence,
            state: state.to_string(),
            explanation,
            suggestion,
        });
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
            // Review prompt (call/0045): until the applied-receipts cross-check
            // lands, the supersession is an unverified hypothesis. The label and
            // the softened lead remedy travel in the prose, because the prose is
            // what a weak agent obeys (W1).
            confidence: Confidence::ReviewPrompt,
            state: String::new(),
            explanation: format!(
                "review prompt: body cites `{room_ref}`; whether the spine superseded it is unverified until the applied-receipts cross-check lands"
            ),
            suggestion: format!(
                "Leave a review note: confirm whether the cited record `{room_ref}` is still current. Only after operator confirmation is the record marked `Status: superseded` via an audited MADR commit; do not act on this prompt alone. Do not edit the memory entry or append to the log."
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
    // Per-user-only by the APPLICABILITY table (the repo format has no
    // per-entry description field); `detect` enforces the row, so no
    // self-guard is needed here.
    let d = input.description.to_lowercase();
    let b = input.body.to_lowercase();
    // description says "not X" but body asserts X
    for neg in ["not ", "no "] {
        if let Some(idx) = d.find(neg) {
            let tail = d[idx + neg.len()..].trim();
            if !tail.is_empty() {
                let token = tail.split_whitespace().next().unwrap_or("");
                if !token.is_empty() && token.len() >= 3 && b.contains(token) {
                    let route = DetectorInput::route_for(input.store);
                    return Some(Finding {
                        entry_slug: input.slug.clone(),
                        store: input.store,
                        kind: "description-body-drift".to_string(),
                        confidence: Confidence::Confirmed,
                        state: String::new(),
                        route,
                        explanation: format!(
                            "description says `{neg}{token}` but the body asserts `{token}`"
                        ),
                        suggestion: suggestion_for(
                            route,
                            &input.slug,
                            "bring the `description:` line back in line with what the body now asserts",
                        ),
                    });
                }
            }
        }
    }
    None
}

/// Stale-state-over-lore: HEURISTIC. A snapshot memory (entry_type = State)
/// whose state language reads "done" but whose body still carries durable
/// measured lore. The detector fires when a State entry's body contains a
/// state-complete signal ("done", "shipped", "complete", "landed",
/// "resolved", "finished", "closed") AND a measured-lore signal (a digit
/// adjacent to a unit-ish character). The operator's fix is a dated
/// current-state block, not a rewrite. The cast review may refine the
/// vocabulary; the current word-lists are the conservative bar.
pub fn detect_stale_state_over_lore(input: &DetectorInput) -> Option<Finding> {
    if input.entry_type != EntryType::State {
        return None;
    }
    let body_lc = input.body.to_lowercase();
    let state_done = ["done", "shipped", "complete", "landed", "resolved", "finished", "closed"];
    // Word-boundary match: the token must be preceded/followed by a non-alphanumeric.
    let has_done = state_done.iter().any(|w| contains_word(&body_lc, w));
    if !has_done {
        return None;
    }
    // measured-lore signal: a digit followed by a non-digit, non-space (a unit
    // character: 'ms', 'kb', 'MB', '%', 'x', etc.). Conservative; the cast
    // review may tighten.
    let has_lore = body_lc
        .chars()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|w| w[0].is_ascii_digit() && !w[1].is_ascii_digit() && !w[1].is_whitespace());
    if !has_lore {
        return None;
    }
    let route = DetectorInput::route_for(input.store);
    Some(Finding {
        entry_slug: input.slug.clone(),
        store: input.store,
        kind: "stale-state-over-lore".to_string(),
        confidence: Confidence::Confirmed,
        state: String::new(),
        route,
        explanation: "State entry reads done but carries durable measured lore; mark the state superseded with a dated current-state block, do not delete the lore".to_string(),
        suggestion: suggestion_for(
            route,
            &input.slug,
            "mark the state superseded with a dated current-state block, keeping the measured lore",
        ),
    })
}

/// Workaround-vs-plan: HEURISTIC, CROSS-ENTRY. Two entries in the same store
/// assert contradictory current facts, neither referencing the other. The
/// detector pairs a `Workaround` entry with a `Fact` entry in the same store
/// whose bodies share a key term (length >= 5, alphanumeric); the missing
/// cross-link is the signal. The cast review may refine the topic-extraction;
/// the current key-term overlap is the conservative bar.
pub fn detect_workaround_vs_plan<'a>(
    entry: &DetectorInput<'a>,
    all: &[DetectorInput<'a>],
) -> Option<Finding> {
    if entry.entry_type != EntryType::Workaround {
        return None;
    }
    let entry_terms = key_terms(&entry.body);
    if entry_terms.is_empty() {
        return None;
    }
    for other in all {
        if other.slug == entry.slug {
            continue;
        }
        if other.entry_type != EntryType::Fact {
            continue;
        }
        if other.store != entry.store {
            continue;
        }
        let other_terms = key_terms(&other.body);
        if entry_terms.intersection(&other_terms).next().is_some() {
            // missing cross-link either way?
            let entry_links_other = entry.body.contains(&format!("[[{}]]", other.slug));
            let other_links_entry = other.body.contains(&format!("[[{}]]", entry.slug));
            if !entry_links_other && !other_links_entry {
                let route = DetectorInput::route_for(entry.store);
                return Some(Finding {
                    entry_slug: entry.slug.clone(),
                    store: entry.store,
                    kind: "workaround-vs-plan".to_string(),
                    confidence: Confidence::Confirmed,
                    state: String::new(),
                    route,
                    explanation: format!(
                        "Workaround entry shares a key term with Fact entry `{}` but neither cross-links the other; one may be a fix-of-the-moment the plan supersedes", other.slug
                    ),
                    suggestion: suggestion_for(
                        route,
                        &entry.slug,
                        &format!("cross-link this entry and `{}` with a `[[link]]`, or supersede whichever is stale", other.slug),
                    ),
                });
            }
        }
    }
    None
}

/// Extract the key terms from a body for topic-overlap matching. A key term
/// is an alphanumeric token of length >= 5 (filters out stop-words like
/// "the", "and", "for"). Lowercased so case-insensitive.
fn key_terms(body: &str) -> BTreeSet<String> {
    body.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 5)
        .map(|t| t.to_lowercase())
        .collect()
}

/// Word-boundary containment: `haystack` contains `needle` as a complete
/// word (the chars before and after the match, if any, are non-alphanumeric).
/// Lowercase both sides for case-insensitive matching.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let needle_lc = needle.to_lowercase();
    let mut start = 0;
    while let Some(idx) = haystack[start..].find(&needle_lc) {
        let abs = start + idx;
        let end = abs + needle_lc.len();
        let before_ok = abs == 0
            || !haystack.as_bytes()[abs - 1].is_ascii_alphanumeric();
        let after_ok = end >= haystack.len()
            || !haystack.as_bytes()[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = abs + 1;
    }
    false
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
        // Index `bytes`, not `body`: on a non-match we advance by one byte, so
        // `i` can land inside a multibyte char (e.g. an em-dash); slicing the
        // `str` there would panic on the non-char-boundary. The prefixes and
        // the digits are ASCII, so a byte-slice check is behaviourally identical.
        let tail = &bytes[i..];
        let (matched_len, kind) = if tail.starts_with(b"call/") {
            (5, "call/")
        } else if tail.starts_with(b"plan/") {
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

/// The dream subcommand entrypoint. Parses args, runs the audit, prints the
/// findings, and exits with the verdict. The thin wrapper around `run_dream`
/// so tests can drive the audit without `process::exit`.
pub fn dream(args: &[String]) {
    let outcome = run_dream(args);
    process::exit(outcome.exit_code);
}

/// The dream audit's outcome: the exit code (`0` clean, `1` findings, `2`
/// cannot-proceed), the findings vector (empty on a clean run), and the
/// `fix_mode` / `json` flags as parsed. Callable from tests; the public
/// `dream` wrapper calls this and exits with the code.
#[allow(dead_code)] // read by tests; fields kept for the test surface
#[derive(Debug)]
pub struct DreamOutcome {
    pub exit_code: i32,
    pub findings: Vec<Finding>,
    pub fix_mode: bool,
    pub json: bool,
}

/// Run the audit over both stores and return the outcome. `--fix` is refused
/// on the repo store (returns exit 2 with the repo-finding count); on the
/// per-user store the structural-safe class would be applied here (currently
/// none; the safe set grows one class at a time per the README, and the first
/// addition lands with cast-review sign-off).
pub fn run_dream(args: &[String]) -> DreamOutcome {
    let mut fix_mode = false;
    let mut json = false;
    let mut retire = false;
    let mut dir: PathBuf = PathBuf::from(".");
    for a in args {
        match a.as_str() {
            "--fix" => fix_mode = true,
            "--json" => json = true,
            "--retire-marker" => retire = true,
            "-h" | "--help" => {
                print_help();
                return DreamOutcome {
                    exit_code: 0,
                    findings: Vec::new(),
                    fix_mode,
                    json,
                };
            }
            other if !other.starts_with("--") => {
                dir = PathBuf::from(other);
            }
            other => {
                eprintln!("host-lifecycle dream: unknown flag {other}");
                return DreamOutcome {
                    exit_code: 2,
                    findings: Vec::new(),
                    fix_mode,
                    json,
                };
            }
        }
    }
    let dir = match fs::canonicalize(&dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("host-lifecycle dream: not a directory: {}: {e}", dir.display());
            return DreamOutcome {
                exit_code: 2,
                findings: Vec::new(),
                fix_mode,
                json,
            };
        }
    };

    // The operator's retirement act (call/0045): only from stamped, and the
    // owed appended correction is named in the output. No audit runs.
    if retire {
        return match retire_marker(&dir) {
            Ok(line) => {
                eprintln!("host-lifecycle {line}");
                DreamOutcome { exit_code: 0, findings: Vec::new(), fix_mode, json }
            }
            Err(e) => {
                eprintln!("host-lifecycle dream: {e}");
                DreamOutcome { exit_code: 2, findings: Vec::new(), fix_mode, json }
            }
        };
    }

    // The stamp (call/0045): dream's sole sanctioned repo-side write, CLI path
    // only, loud so the operator commits it.
    let home = env::var_os("HOME").map(PathBuf::from);
    if let Some(line) = stamp_if_observed(&dir, home.as_deref()) {
        eprintln!("host-lifecycle {line}");
    }

    let audit = run_audit(&dir);
    let findings = audit.findings;

    if fix_mode {
        let repo_count = findings.iter().filter(|f| f.store == StoreLoc::Repo).count();
        if repo_count > 0 {
            eprintln!(
                "host-lifecycle dream: --fix refuses the repo store (the append-only tier); {repo_count} repo finding(s) reported, not applied"
            );
            return DreamOutcome {
                exit_code: 2,
                findings,
                fix_mode,
                json,
            };
        }
        // The structural-safe class on the per-user store lands with cast-review
        // sign-off; for now --fix is a no-op that confirms the audit ran.
        eprintln!(
            "host-lifecycle dream: --fix mode acknowledged; no structural-safe classes are auto-applied yet (the safe set grows one class at a time)"
        );
    }

    if json {
        print_json(&findings, &audit.marker, audit.store_present, audit.history_checked);
    } else {
        print_text(&findings, &audit.marker, audit.store_present, audit.history_checked);
    }

    // The exit split (call/0045): clean 0; review prompts alone are the
    // advisory tier at 3 (mirroring host-lint's warn); any confirmed finding
    // gates at 1.
    let confirmed = findings
        .iter()
        .filter(|f| f.confidence == Confidence::Confirmed)
        .count();
    DreamOutcome {
        exit_code: if findings.is_empty() {
            0
        } else if confirmed > 0 {
            1
        } else {
            3
        },
        findings,
        fix_mode,
        json,
    }
}

/// The repo-side tier marker file (call/0045): the per-user tier's declared
/// in-use state. Tool-written on first observed store (the CLI path stamps
/// it); retired by the operator (`--retire-marker`); never silently
/// re-stamped.
pub const TIER_MARKER_FILE: &str = ".host-memory-tier";

/// The parsed tier marker: the declared state plus its provenance line (date
/// and author of the stamp or the retirement), carried into coverage lines
/// and finding explanations so a possibly-stale latch is judgeable at the
/// same glance as the finding.
pub struct TierMarker {
    pub state: TierState,
    pub provenance: String,
}

/// Read the tier marker from the project root. An absent or unreadable file
/// is the Absent state: failing toward Absent is loud (the confirmed tier),
/// where failing toward Stamped would silently defuse the teeth.
pub fn read_tier_marker(project_dir: &Path) -> TierMarker {
    let Ok(content) = fs::read_to_string(project_dir.join(TIER_MARKER_FILE)) else {
        return TierMarker { state: TierState::Absent, provenance: String::new() };
    };
    let mut state = TierState::Absent;
    let mut stamped = String::new();
    let mut retired = String::new();
    for line in content.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("status") {
            match v.trim_start_matches([' ', '=']).trim() {
                "stamped" => state = TierState::Stamped,
                "retired" => state = TierState::Retired,
                _ => {}
            }
        } else if let Some(v) = line.strip_prefix("stamped") {
            stamped = v.trim_start_matches([' ', '=']).trim().to_string();
        } else if let Some(v) = line.strip_prefix("retired") {
            retired = v.trim_start_matches([' ', '=']).trim().to_string();
        }
    }
    let provenance = match state {
        TierState::Retired => retired,
        _ => stamped,
    };
    TierMarker { state, provenance }
}

/// Stamp the tier marker if an initialized per-user store is observed on this
/// machine and the marker is absent (call/0045): dream's sole sanctioned
/// repo-side write, performed only in the CLI path. Returns the loud line to
/// print so the operator commits the stamp (operator-attributable: the tool
/// writes the file, the operator commits it). No-op under any other marker
/// state; retirement is never overwritten (the contradiction finding carries
/// that case).
pub fn stamp_if_observed(project_dir: &Path, home: Option<&Path>) -> Option<String> {
    let marker = read_tier_marker(project_dir);
    if marker.state != TierState::Absent {
        return None;
    }
    let store = home.and_then(|h| MemoryStore::open(&h.join(".host-memory"), project_dir).ok())?;
    if !store.initialized() {
        return None;
    }
    let provenance = provenance_line(project_dir);
    let content = format!(
        "# The per-user memory tier's in-use marker (call/0045). Tool-written by\n\
         # `host-lifecycle dream` when it first observes an initialized store on a\n\
         # machine; retire via `host-lifecycle dream --retire-marker` (an operator\n\
         # act, recorded as an appended MEMORY.md correction). Never hand-edit.\n\
         status  = stamped\n\
         stamped = {provenance}\n"
    );
    if fs::write(project_dir.join(TIER_MARKER_FILE), content).is_err() {
        return None;
    }
    Some(format!(
        "dream: stamped the per-user tier marker ({TIER_MARKER_FILE}, {provenance}); commit it — the tier is now declared in use"
    ))
}

/// Retire the tier marker: the operator's act, invoked via `--retire-marker`.
/// Only from the stamped state; the returned line names the appended
/// correction the operator owes. Refuses when there is nothing to retire.
pub fn retire_marker(project_dir: &Path) -> Result<String, String> {
    let marker = read_tier_marker(project_dir);
    match marker.state {
        TierState::Stamped => {}
        TierState::Absent => return Err("no tier marker to retire (the tier was never declared in use)".to_string()),
        TierState::Retired => return Err(format!("the tier marker is already retired ({})", marker.provenance)),
    }
    let provenance = provenance_line(project_dir);
    let content = format!(
        "# The per-user memory tier's in-use marker (call/0045). RETIRED: the\n\
         # operator declared the tier out of use. A store observed after retirement\n\
         # is a contradiction finding, never a silent re-stamp. Never hand-edit.\n\
         status  = retired\n\
         stamped = {}\n\
         retired = {provenance}\n",
        marker.provenance
    );
    fs::write(project_dir.join(TIER_MARKER_FILE), content)
        .map_err(|e| format!("cannot write {TIER_MARKER_FILE}: {e}"))?;
    Ok(format!(
        "dream: retired the per-user tier marker ({provenance}). Record the retirement as a new dated MEMORY.md entry and commit both; unresolved cross-store links re-tier confirmed from here"
    ))
}

/// The stamp/retirement provenance: date plus the git author, so the marker is
/// operator-attributable and its age judgeable at a glance.
fn provenance_line(project_dir: &Path) -> String {
    let date = std::process::Command::new("date")
        .arg("+%F")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown-date".to_string());
    let who = std::process::Command::new("git")
        .arg("-C")
        .arg(project_dir)
        .args(["config", "user.email"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-author".to_string());
    format!("{date} {who}")
}

/// One audit's full result: the findings plus the store facts the coverage
/// lines and callers report from, so output can never claim more than the
/// audit did (call/0045 legibility).
pub struct Audit {
    pub findings: Vec<Finding>,
    pub marker: TierMarker,
    pub store_present: bool,
    /// Whether the append-only history scan actually ran (a git history was
    /// available and `git log` succeeded). False means unchecked, and the
    /// coverage line says so instead of claiming the check.
    pub history_checked: bool,
}

/// The full audit. Reads the per-user store (memory.rs), the repo `MEMORY.md`,
/// and the tier marker; resolves links against the union of both stores; runs
/// the detector engine over each entry; returns findings in
/// detector-declaration order, per-user tier first.
pub fn run_audit(project_dir: &Path) -> Audit {
    let home = env::var_os("HOME").map(PathBuf::from);
    run_audit_with_home(project_dir, home.as_deref())
}

/// The audit core with an explicit home (testable without env mutation). Pure
/// read-side: the marker STAMP is a write and happens only in the CLI path,
/// so the MCP consolidate surface stays read-only. Detection treats an
/// observed-but-unstamped store as in use; the stamp is the durable record
/// for other machines, not the detection input.
pub fn run_audit_with_home(project_dir: &Path, home: Option<&Path>) -> Audit {
    let mut findings = Vec::new();
    let marker = read_tier_marker(project_dir);

    // Per-user store: `open` always succeeds (lazy materialization), so the
    // presence signal is an initialized store (index present), never a bare
    // directory.
    let per_user_store = home.and_then(|h| MemoryStore::open(&h.join(".host-memory"), project_dir).ok());
    let store_present = per_user_store.as_ref().map(|s| s.initialized()).unwrap_or(false);
    let per_user_entries = match &per_user_store {
        Some(store) if store_present => store.list().unwrap_or_default(),
        _ => Vec::new(),
    };

    // The union slug set (call/0045): a [[link]] resolves against the whole
    // memory graph, not one tier.
    let repo_memory = project_dir.join("MEMORY.md");
    let repo_content = fs::read_to_string(&repo_memory).unwrap_or_default();
    let repo_entries = parse_repo_memory_sections(&repo_content);
    let mut union_slugs: BTreeSet<String> = per_user_entries.iter().map(|e| e.slug.clone()).collect();
    for e in &repo_entries {
        union_slugs.insert(e.slug.clone());
    }

    // The marker contradiction (call/0045): a store observed after retirement
    // surfaces as a confirmed finding, never a silent re-stamp.
    if store_present && marker.state == TierState::Retired {
        findings.push(marker_contradiction_finding(&marker));
    }

    let per_user_inputs: Vec<DetectorInput> = per_user_entries
        .iter()
        .map(|entry| DetectorInput {
            slug: entry.slug.clone(),
            description: entry.description.clone(),
            body: entry.body.clone(),
            superseded_by: entry.superseded_by.clone(),
            entry_type: entry.entry_type.clone(),
            store: StoreLoc::PerUser,
            store_dir: per_user_store.as_ref().map(|s| s.dir()),
            known_slugs: &union_slugs,
            tier: marker.state,
            tier_provenance: &marker.provenance,
            store_present,
        })
        .collect();
    for input in &per_user_inputs {
        findings.extend(detect(input));
    }
    findings.extend(detect_cross(&per_user_inputs));

    // Repo MEMORY.md (the append-only tier), through the same engine and the
    // same union; the applicability table decides which detectors run here.
    for entry in &repo_entries {
        let input = DetectorInput {
            slug: entry.slug.clone(),
            description: entry.heading.clone(),
            body: entry.body.clone(),
            superseded_by: String::new(),
            entry_type: EntryType::Fact,
            store: StoreLoc::Repo,
            store_dir: None,
            known_slugs: &union_slugs,
            tier: marker.state,
            tier_provenance: &marker.provenance,
            store_present,
        };
        findings.extend(detect(&input));
    }
    // Append-only-violation: scan git history of MEMORY.md for in-place body
    // edits to existing sections. Returns one finding per edited section plus
    // whether the scan actually ran.
    let (violations, history_checked) = detect_append_only_violations(&repo_memory);
    findings.extend(violations);
    Audit {
        findings,
        marker,
        store_present,
        history_checked,
    }
}

/// The marker-contradiction finding. The spec's `Finding.entry` is null here;
/// the printed locus is the marker itself.
fn marker_contradiction_finding(marker: &TierMarker) -> Finding {
    Finding {
        entry_slug: "tier-marker".to_string(),
        store: StoreLoc::Repo,
        kind: "marker-contradiction".to_string(),
        route: Route::Append,
        confidence: Confidence::Confirmed,
        state: String::new(),
        explanation: format!(
            "a per-user store is present on this machine but the tier marker is retired ({})",
            marker.provenance
        ),
        suggestion: "The tier marker was retired but a per-user store exists here. The operator either re-opts in (append a correction that records the decision, remove the retired marker, and let the next dream run re-stamp) or removes the per-user store. Never edit the marker to stamped by hand.".to_string(),
    }
}

/// Append-only-violation: HEURISTIC, HISTORY-BASED. An in-place edit to an
/// existing repo MEMORY.md entry (a section whose body changed between two
/// revisions without a new dated heading). The detector runs `git log -p` on
/// MEMORY.md and parses the diff: a hunk whose removed lines are inside an
/// existing `### ` section (not the section heading itself, not a new
/// section's body) is an in-place edit. Conservative: the cast review may
/// refine the heuristic; for now it flags any section with removed body
/// lines across revisions.
fn detect_append_only_violations(path: &Path) -> (Vec<Finding>, bool) {
    let mut out = Vec::new();
    let Some(parent) = path.parent() else {
        return (out, false);
    };
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("MEMORY.md");
    // `git log -p --no-color -- MEMORY.md` under the project dir. A failed or
    // unavailable history is reported as unchecked, never as a clean check.
    let result = std::process::Command::new("git")
        .arg("-C")
        .arg(parent)
        .args(["log", "-p", "--no-color", "--", file_name])
        .output();
    let Ok(output) = result else {
        return (out, false);
    };
    if !output.status.success() {
        return (out, false);
    }
    let log = String::from_utf8_lossy(&output.stdout);
    // Track sections that have removals inside them in any revision's diff.
    // A `### ` heading line (any prefix: context, +, or -) marks the current
    // section; a `-` body line (not a heading, not a meta line) inside a known
    // section is the in-place-edit signal.
    let mut edited_sections: BTreeSet<String> = BTreeSet::new();
    let mut current_section: Option<String> = None;
    let mut in_diff = false;
    for line in log.lines() {
        if line.starts_with("diff --git") || line.starts_with("commit ") {
            in_diff = false;
            current_section = None;
            continue;
        }
        if line.starts_with("@@") {
            in_diff = true;
            current_section = None;
            continue;
        }
        if !in_diff {
            continue;
        }
        // Inside a diff hunk. Recognise section headings under any prefix.
        // Context heading ` ### X`, added heading `+### X`, removed heading
        // `-### X`. A removed heading is a section rename/deletion (not an
        // in-place edit); reset so body removals under it are not attributed.
        if let Some(heading) = line
            .strip_prefix(" ### ")
            .or_else(|| line.strip_prefix("+### "))
        {
            current_section = Some(heading.trim().to_string());
            continue;
        }
        if line.strip_prefix("-### ").is_some() {
            current_section = None;
            continue;
        }
        // A removal inside the current section's body. Skip the `---` file
        // meta line (which `strip_prefix('-')` would otherwise catch).
        if line.starts_with('-') && !line.starts_with("---") {
            if let Some(section) = &current_section {
                edited_sections.insert(section.clone());
            }
        }
    }
    for section in edited_sections {
        out.push(Finding {
            entry_slug: slugify(&section),
            store: StoreLoc::Repo,
            kind: "append-only-violation".to_string(),
            confidence: Confidence::Confirmed,
            state: String::new(),
            route: Route::Append,
            explanation: format!(
                "section `{section}` has body removals in git history; the repo MEMORY.md is append-only (CLAUDE.md section 6); if the body changed, append a correction with a forward [[link]] instead"
            ),
            // Bespoke: an append-only violation is already an in-place edit; the
            // imperative is to restore and append, never to edit further.
            suggestion: format!(
                "The repo MEMORY.md section `{section}` was edited in place — a CLAUDE.md §6 violation. Append a new dated correction that restores the removed text and names the violated section with a forward `[[link]]`; the restoration rides the append. Never edit repo entries in place."
            ),
        });
    }
    (out, true)
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
    eprintln!("usage: host-lifecycle dream [<dir>] [--fix] [--json] [--retire-marker]");
    eprintln!("  --fix            refuse the repo store; apply the structural-safe class to per-user (none yet)");
    eprintln!("  --json           machine-readable findings");
    eprintln!("  --retire-marker  operator act: declare the per-user tier out of use (record the appended correction)");
    eprintln!("  -h, --help       this help");
}

/// The per-tier coverage lines (call/0045): generated from the same
/// APPLICABILITY rows and store facts the engine enforced, so the output and
/// the engine cannot disagree. Printed in the clean case and the findings
/// case alike; the marker's provenance rides along so a possibly-stale latch
/// is judgeable in the same glance.
pub fn coverage_lines(marker: &TierMarker, store_present: bool, history_checked: bool) -> Vec<String> {
    let marker_desc = match marker.state {
        TierState::Absent => "tier marker: absent".to_string(),
        TierState::Stamped => format!("tier marker: stamped {}", marker.provenance),
        TierState::Retired => format!("tier marker: retired {}", marker.provenance),
    };
    let mut out = Vec::new();
    let repo_ran: Vec<&str> = APPLICABILITY
        .iter()
        .filter(|r| r.1.contains(&StoreLoc::Repo))
        .map(|r| r.0)
        // The history scan only counts as checked when it actually ran; a
        // failed or absent git history moves it to the unchecked clause.
        .filter(|k| history_checked || *k != "append-only-violation")
        .collect();
    let repo_skipped: Vec<String> = APPLICABILITY
        .iter()
        .filter(|r| !r.1.contains(&StoreLoc::Repo))
        .map(|r| format!("{} ({})", r.0, r.2))
        .collect();
    let mut repo_line = format!(
        "coverage: repo tier: checked {}; links resolved against the union of both stores ({marker_desc})",
        repo_ran.join(", ")
    );
    if !history_checked {
        repo_line.push_str("; unchecked: append-only-violation (no git history available)");
    }
    if !repo_skipped.is_empty() {
        repo_line.push_str(&format!("; not applicable: {}", repo_skipped.join("; ")));
    }
    out.push(repo_line);
    if store_present {
        let per_user_ran: Vec<&str> = APPLICABILITY
            .iter()
            .filter(|r| r.1.contains(&StoreLoc::PerUser))
            .map(|r| r.0)
            .collect();
        out.push(format!(
            "coverage: per-user tier: store present on this machine; checked {}",
            per_user_ran.join(", ")
        ));
    } else {
        out.push(format!(
            "coverage: per-user tier: store absent on this machine ({marker_desc}); cross-store links resolved against the repo log only"
        ));
    }
    out
}

/// The per-state dangling-link counts (call/0045 condition 6): a new dangle
/// must not hide inside a standing baseline, so the three states count
/// separately wherever the totals print.
fn dangling_state_counts(findings: &[Finding]) -> (usize, usize, usize) {
    let count = |s: &str| findings.iter().filter(|f| f.state == s).count();
    (count("tier-unused"), count("uninitialized-here"), count("entry-missing"))
}

fn print_text(findings: &[Finding], marker: &TierMarker, store_present: bool, history_checked: bool) {
    let confirmed: Vec<&Finding> = findings.iter().filter(|f| f.confidence == Confidence::Confirmed).collect();
    let prompts: Vec<&Finding> = findings.iter().filter(|f| f.confidence == Confidence::ReviewPrompt).collect();
    if findings.is_empty() {
        println!("dream: clean");
    } else {
        // Grouped presentation (call/0045): the count line leads, confirmed
        // findings print above the review prompts, and each line carries its
        // confidence so the label cannot be missed.
        println!(
            "dream: {} confirmed finding(s), {} review prompt(s)",
            confirmed.len(),
            prompts.len()
        );
        let (tier_unused, uninit_here, entry_missing) = dangling_state_counts(findings);
        if tier_unused + uninit_here + entry_missing > 0 {
            println!(
                "dangling-link states: {tier_unused} tier-unused, {uninit_here} uninitialized-here, {entry_missing} entry-missing"
            );
        }
        for f in confirmed.iter().chain(prompts.iter()) {
            println!(
                "{} ({}) [{}] {} route={} — {}",
                f.entry_slug,
                f.store.as_str(),
                f.kind,
                f.confidence.as_str(),
                f.route.as_str(),
                f.explanation
            );
            println!("  → {}", f.suggestion);
        }
    }
    for line in coverage_lines(marker, store_present, history_checked) {
        println!("{line}");
    }
    if !findings.is_empty() {
        eprintln!(
            "host-lifecycle dream: {} finding(s) across the memory stores",
            findings.len()
        );
    }
}

fn print_json(findings: &[Finding], marker: &TierMarker, store_present: bool, history_checked: bool) {
    // An object, not a bare array (call/0045): [] cannot distinguish clean
    // from unchecked, so the coverage travels in-band. The shape change is
    // ledgered for adopters in the release's migration entry.
    let mut s = String::from("{\n  \"findings\": [\n");
    for (i, f) in findings.iter().enumerate() {
        s.push_str(&format!(
            "    {{\"entry\": \"{}\", \"store\": \"{}\", \"kind\": \"{}\", \"confidence\": \"{}\", \"state\": \"{}\", \"route\": \"{}\", \"explanation\": \"{}\", \"suggestion\": \"{}\"}}",
            json_escape(&f.entry_slug),
            f.store.as_str(),
            f.kind,
            f.confidence.as_str(),
            json_escape(&f.state),
            f.route.as_str(),
            json_escape(&f.explanation),
            json_escape(&f.suggestion)
        ));
        if i + 1 < findings.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("  ],\n");
    let marker_state = match marker.state {
        TierState::Absent => "absent",
        TierState::Stamped => "stamped",
        TierState::Retired => "retired",
    };
    let (tier_unused, uninit_here, entry_missing) = dangling_state_counts(findings);
    s.push_str(&format!(
        "  \"coverage\": {{\"marker\": {{\"state\": \"{}\", \"provenance\": \"{}\"}}, \"per_user_store_present\": {}, \"history_checked\": {}, \"dangling_states\": {{\"tier_unused\": {}, \"uninitialized_here\": {}, \"entry_missing\": {}}}, \"tiers\": [",
        marker_state,
        json_escape(&marker.provenance),
        store_present,
        history_checked,
        tier_unused,
        uninit_here,
        entry_missing
    ));
    let tiers = coverage_lines(marker, store_present, history_checked);
    for (i, t) in tiers.iter().enumerate() {
        s.push_str(&format!("\"{}\"", json_escape(t)));
        if i + 1 < tiers.len() {
            s.push_str(", ");
        }
    }
    s.push_str("]}\n}");
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
            tier: TierState::Stamped,
            tier_provenance: "",
            store_present: true,
        }
    }

    fn repo_input<'a>(slug: &str, desc: &str, body: &str, slugs: &'a BTreeSet<String>) -> DetectorInput<'a> {
        DetectorInput { store: StoreLoc::Repo, ..input(slug, desc, body, slugs) }
    }

    fn tmp_fixture(name: &str, memory_md: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("dream-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("MEMORY.md"), memory_md).unwrap();
        dir
    }

    // --- Tier-marker lifecycle (plan/0076 #implement-marker) ---

    fn tmp_dir(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("dream-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn materialized_store(home: &Path, proj: &Path) -> PathBuf {
        let store = MemoryStore::open(&home.join(".host-memory"), proj).unwrap();
        fs::create_dir_all(store.dir()).unwrap();
        // An initialized store means the index exists (a bare directory does
        // not count, the anti-gaming bar).
        fs::write(store.dir().join("MEMORY.md"), "").unwrap();
        store.dir().to_path_buf()
    }

    #[test]
    fn marker_stamps_on_observed_store() {
        let proj = tmp_fixture("stamp", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("stamp-home");
        // No store yet: no stamp.
        assert!(stamp_if_observed(&proj, Some(&home)).is_none());
        assert_eq!(read_tier_marker(&proj).state, TierState::Absent);
        // Store observed: stamp, with provenance, loudly.
        materialized_store(&home, &proj);
        let msg = stamp_if_observed(&proj, Some(&home)).expect("stamp on observed store");
        assert!(msg.contains(TIER_MARKER_FILE), "message names the file: {msg}");
        let marker = read_tier_marker(&proj);
        assert_eq!(marker.state, TierState::Stamped);
        assert!(!marker.provenance.is_empty());
        // A second run does not re-stamp.
        assert!(stamp_if_observed(&proj, Some(&home)).is_none());
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn marker_retires_from_stamped() {
        let proj = tmp_fixture("retire", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("retire-home");
        // Nothing to retire before any stamp: refuse.
        assert!(retire_marker(&proj).is_err());
        materialized_store(&home, &proj);
        stamp_if_observed(&proj, Some(&home)).expect("stamp");
        let line = retire_marker(&proj).expect("retire from stamped");
        assert!(line.contains("MEMORY.md"), "names the owed appended correction: {line}");
        let marker = read_tier_marker(&proj);
        assert_eq!(marker.state, TierState::Retired);
        assert!(!marker.provenance.is_empty());
        // Already retired: refuse; and no silent re-stamp.
        assert!(retire_marker(&proj).is_err());
        assert!(stamp_if_observed(&proj, Some(&home)).is_none());
        assert_eq!(read_tier_marker(&proj).state, TierState::Retired);
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn marker_contradiction_after_retirement() {
        let proj = tmp_fixture("contradict", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("contradict-home");
        let store_dir = materialized_store(&home, &proj);
        stamp_if_observed(&proj, Some(&home)).expect("stamp");
        retire_marker(&proj).expect("retire");
        // Store still present after retirement: confirmed contradiction finding.
        let findings = run_audit_with_home(&proj, Some(&home)).findings;
        let f = findings
            .iter()
            .find(|f| f.kind == "marker-contradiction")
            .expect("contradiction finding");
        assert_eq!(f.confidence, Confidence::Confirmed);
        assert!(f.suggestion.contains("Never edit the marker"), "anti-hand-edit tail: {}", f.suggestion);
        // Store removed: the contradiction clears.
        fs::remove_dir_all(&store_dir).unwrap();
        let findings = run_audit_with_home(&proj, Some(&home)).findings;
        assert!(!findings.iter().any(|f| f.kind == "marker-contradiction"));
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    // --- The write-tests matrix (plan/0076): states, kinds, exits ---

    #[test]
    fn marker_transition_absent_to_stamped() {
        let proj = tmp_fixture("edge-stamp", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("edge-stamp-home");
        materialized_store(&home, &proj);
        assert_eq!(read_tier_marker(&proj).state, TierState::Absent);
        stamp_if_observed(&proj, Some(&home)).expect("stamp");
        assert_eq!(read_tier_marker(&proj).state, TierState::Stamped);
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn marker_transition_stamped_to_retired() {
        let proj = tmp_fixture("edge-retire", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("edge-retire-home");
        materialized_store(&home, &proj);
        stamp_if_observed(&proj, Some(&home)).expect("stamp");
        retire_marker(&proj).expect("retire");
        assert_eq!(read_tier_marker(&proj).state, TierState::Retired);
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn marker_stamp_only_from_absent() {
        let proj = tmp_fixture("stamp-guard", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("stamp-guard-home");
        materialized_store(&home, &proj);
        stamp_if_observed(&proj, Some(&home)).expect("first stamp");
        // Stamped: no re-stamp. Retired: no re-stamp either.
        assert!(stamp_if_observed(&proj, Some(&home)).is_none());
        retire_marker(&proj).expect("retire");
        assert!(stamp_if_observed(&proj, Some(&home)).is_none());
        assert_eq!(read_tier_marker(&proj).state, TierState::Retired);
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn marker_retire_only_from_stamped() {
        let proj = tmp_fixture("retire-guard", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("retire-guard-home");
        // Absent: refuse.
        assert!(retire_marker(&proj).is_err());
        materialized_store(&home, &proj);
        stamp_if_observed(&proj, Some(&home)).expect("stamp");
        retire_marker(&proj).expect("retire from stamped");
        // Retired: refuse again.
        assert!(retire_marker(&proj).is_err());
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn marker_no_contradiction_before_retirement() {
        let proj = tmp_fixture("no-contradict", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("no-contradict-home");
        materialized_store(&home, &proj);
        stamp_if_observed(&proj, Some(&home)).expect("stamp");
        let findings = run_audit_with_home(&proj, Some(&home)).findings;
        assert!(!findings.iter().any(|f| f.kind == "marker-contradiction"));
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn marker_contradiction_finding_created() {
        let proj = tmp_fixture("contradict-shape", "# M\n\n### E\n\nClean.\n");
        let home = tmp_dir("contradict-shape-home");
        materialized_store(&home, &proj);
        stamp_if_observed(&proj, Some(&home)).expect("stamp");
        retire_marker(&proj).expect("retire");
        let findings = run_audit_with_home(&proj, Some(&home)).findings;
        let f = findings.iter().find(|f| f.kind == "marker-contradiction").expect("finding");
        assert_eq!(f.entry_slug, "tier-marker");
        assert_eq!(f.store, StoreLoc::Repo);
        assert_eq!(f.route, Route::Append);
        assert_eq!(f.confidence, Confidence::Confirmed);
        let _ = fs::remove_dir_all(&proj);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn drift_finding_created_per_user_only() {
        // The APPLICABILITY row (repo has no per-entry description) is the
        // enforcement: the same drifting text fires per-user and not on repo.
        let slugs = known(&["alpha"]);
        let per_user = input("alpha", "not stabilized yet", "The API is stabilized.", &slugs);
        let f = detect(&per_user)
            .into_iter()
            .find(|f| f.kind == "description-body-drift")
            .expect("per-user drift finding");
        assert_eq!(f.confidence, Confidence::Confirmed);
        assert_eq!(f.route, Route::Edit);
        let repo = repo_input("alpha", "not stabilized yet", "The API is stabilized.", &slugs);
        assert!(!detect(&repo).iter().any(|f| f.kind == "description-body-drift"));
    }

    #[test]
    fn dangling_confirmed_without_marker() {
        let slugs = known(&["alpha"]);
        let mut inp = repo_input("alpha", "x", "See [[missing-concept]].", &slugs);
        inp.tier = TierState::Absent;
        inp.store_present = false;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("dangling finding");
        assert_eq!(f.confidence, Confidence::Confirmed);
        // Retirement re-escalates to confirmed too (the recorded pressure valve).
        let mut retired = repo_input("alpha", "x", "See [[missing-concept]].", &slugs);
        retired.tier = TierState::Retired;
        retired.store_present = false;
        let f2 = detect(&retired)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("dangling finding under retirement");
        assert_eq!(f2.confidence, Confidence::Confirmed);
    }

    #[test]
    fn dangling_not_confirmed_when_marker_present() {
        let slugs = known(&["alpha"]);
        let mut inp = repo_input("alpha", "x", "See [[missing-concept]].", &slugs);
        inp.tier = TierState::Stamped;
        inp.store_present = false;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("dangling finding");
        assert_eq!(f.confidence, Confidence::ReviewPrompt);
    }

    #[test]
    fn dangling_confirmed_finding_created() {
        let slugs = known(&["alpha"]);
        let mut inp = repo_input("alpha", "x", "See [[missing-concept]].", &slugs);
        inp.tier = TierState::Absent;
        inp.store_present = false;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("finding");
        assert_eq!(f.route, Route::Append);
        // The remedy leads with the operator's initialization fork and keeps
        // the append-only anti-action tail (call/0045 guardrails).
        assert!(f.suggestion.starts_with("If a per-user store is intended, the operator initializes"),
            "initialization fork must lead: {}", f.suggestion);
        assert!(f.suggestion.contains("appended correction"), "append-only action: {}", f.suggestion);
        assert!(f.suggestion.contains("Never edit the existing entry in place"), "anti-edit tail: {}", f.suggestion);
    }

    #[test]
    fn dangling_advisory_when_store_absent_here() {
        let slugs = known(&["alpha"]);
        let mut inp = repo_input("alpha", "x", "See [[missing-concept]].", &slugs);
        inp.tier = TierState::Stamped;
        inp.tier_provenance = "2026-07-22 t@t";
        inp.store_present = false;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("finding");
        assert_eq!(f.confidence, Confidence::ReviewPrompt);
        // The lead action is the safe no-op, and a drop on one machine's
        // evidence is forbidden in the string itself.
        assert!(f.suggestion.starts_with("Leave the"), "safe no-op leads: {}", f.suggestion);
        assert!(f.suggestion.contains("Do not drop the link on this machine's evidence alone"),
            "machine-local drop forbidden: {}", f.suggestion);
    }

    #[test]
    fn dangling_uninitialized_requires_marker() {
        // The uninitialized-here shape exists only under a stamped marker:
        // with the marker absent the same evidence is the confirmed state.
        let slugs = known(&["alpha"]);
        let mut inp = repo_input("alpha", "x", "See [[missing-concept]].", &slugs);
        inp.tier = TierState::Absent;
        inp.store_present = false;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("finding");
        assert!(!f.suggestion.starts_with("Leave the"), "not the uninitialized shape: {}", f.suggestion);
        assert_eq!(f.confidence, Confidence::Confirmed);
    }

    #[test]
    fn dangling_uninitialized_finding_created() {
        let slugs = known(&["alpha"]);
        let mut inp = repo_input("alpha", "x", "See [[missing-concept]].", &slugs);
        inp.tier = TierState::Stamped;
        inp.tier_provenance = "2026-07-22 t@t";
        inp.store_present = false;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("finding");
        assert_eq!(f.route, Route::Append);
        assert!(f.explanation.contains("per-user store absent here"), "state named: {}", f.explanation);
        assert!(f.explanation.contains("stamped 2026-07-22 t@t"), "provenance travels: {}", f.explanation);
    }

    #[test]
    fn dangling_advisory_when_entry_missing() {
        // Default helper context: stamped marker, store present.
        let slugs = known(&["alpha"]);
        let per_user = input("alpha", "x", "See [[beta]] which is missing.", &slugs);
        let f = detect(&per_user)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("finding");
        assert_eq!(f.confidence, Confidence::ReviewPrompt);
        assert!(f.suggestion.starts_with("Edit"), "per-user entry-missing edits in place: {}", f.suggestion);
    }

    #[test]
    fn dangling_confirmed_only_when_tier_unused() {
        // The invariant sweep: across the state matrix, confirmed dangling
        // holds only while the tier is not in use (absent or retired).
        let slugs = known(&["alpha"]);
        let matrix = [
            (TierState::Absent, false, Confidence::Confirmed),
            (TierState::Retired, false, Confidence::Confirmed),
            (TierState::Retired, true, Confidence::Confirmed),
            (TierState::Stamped, false, Confidence::ReviewPrompt),
            (TierState::Stamped, true, Confidence::ReviewPrompt),
            (TierState::Absent, true, Confidence::ReviewPrompt),
        ];
        for (tier, present, expect) in matrix {
            let mut inp = repo_input("alpha", "x", "See [[missing-concept]].", &slugs);
            inp.tier = tier;
            inp.store_present = present;
            let f = detect(&inp)
                .into_iter()
                .find(|f| f.kind == "dangling-link")
                .expect("finding");
            assert_eq!(f.confidence, expect, "state ({tier:?}, store_present={present})");
            // The machine-readable state token matches the confidence tier.
            match expect {
                Confidence::Confirmed => assert_eq!(f.state, "tier-unused"),
                Confidence::ReviewPrompt => assert!(
                    f.state == "uninitialized-here" || f.state == "entry-missing",
                    "advisory state token: {}",
                    f.state
                ),
            }
        }
    }

    #[test]
    fn dangling_retired_per_user_routes_edit_not_append() {
        // The contradiction scenario (retired marker, store still present): a
        // per-user entry's confirmed dangle must ride the store-correct edit
        // frame, never a repo-append imperative (the W1 failure class).
        let slugs = known(&["alpha"]);
        let mut inp = input("alpha", "x", "See [[beta]] which is missing.", &slugs);
        inp.tier = TierState::Retired;
        inp.tier_provenance = "2026-07-22 t@t";
        inp.store_present = true;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("finding");
        assert_eq!(f.confidence, Confidence::Confirmed);
        assert_eq!(f.state, "tier-unused");
        assert_eq!(f.route, Route::Edit);
        assert!(f.suggestion.starts_with("Edit"), "store-correct edit frame: {}", f.suggestion);
        assert!(f.suggestion.contains("Do not append"), "anti-append tail: {}", f.suggestion);
        // The retired remedy names the re-opt-in protocol, never the
        // initialize-and-commit fork (which is false from retired).
        assert!(f.suggestion.contains("removes the retired marker"), "re-opt-in protocol: {}", f.suggestion);
        assert!(!f.suggestion.contains("commits the tier marker"), "no stale fork: {}", f.suggestion);
        assert!(f.suggestion.contains("never the auditing agent's act"), "anti-stamp tail: {}", f.suggestion);
    }

    #[test]
    fn superseded_unlinked_finding_created() {
        // Per-user single-branch creation shape: route edit, confirmed, the
        // W1 edit frame with its anti-append tail.
        let slugs = known(&["alpha", "beta"]);
        let mut inp = input("alpha", "older", "Alpha body. See nothing.", &slugs);
        inp.superseded_by = "beta".to_string();
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "superseded-but-unlinked")
            .expect("finding");
        assert_eq!(f.route, Route::Edit);
        assert_eq!(f.confidence, Confidence::Confirmed);
        assert!(f.suggestion.starts_with("Edit"), "edit frame: {}", f.suggestion);
    }

    #[test]
    fn stale_state_finding_created() {
        let slugs = known(&["snapshot"]);
        let mut inp = input(
            "snapshot",
            "GPU parity status",
            "Single-GPU parity is done; the dual-GPU measurement was 42ms.",
            &slugs,
        );
        inp.entry_type = EntryType::State;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "stale-state-over-lore")
            .expect("finding");
        assert_eq!(f.route, Route::Edit);
        assert_eq!(f.confidence, Confidence::Confirmed);
    }

    #[test]
    fn workaround_finding_created() {
        let slugs = known(&["workaround", "plan"]);
        let mut w = input(
            "workaround",
            "k=0 kernel fix",
            "The k=0_kernel needs the AR16 model applied at boot.",
            &slugs,
        );
        w.entry_type = EntryType::Workaround;
        let mut p = input(
            "plan",
            "kernel plan",
            "The k=0_kernel lands properly in the boot path rework.",
            &slugs,
        );
        p.entry_type = EntryType::Fact;
        let all = vec![w, p];
        let f = detect_cross(&all)
            .into_iter()
            .find(|f| f.kind == "workaround-vs-plan")
            .expect("finding");
        assert_eq!(f.route, Route::Edit);
        assert_eq!(f.confidence, Confidence::Confirmed);
    }

    #[test]
    fn dream_confirmed_latch_sets() {
        // A confirmed finding pushes the verdict to findings (exit 1).
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee [[missing-note]] here.\n", false);
        assert!(outcome.findings.iter().any(|f| f.confidence == Confidence::Confirmed));
        assert_eq!(outcome.exit_code, 1);
    }

    #[test]
    fn dream_confirmed_latch_guarded() {
        // Review prompts alone never set the confirmed latch: advisory exit.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee call/0017.\n", false);
        assert!(!outcome.findings.iter().any(|f| f.confidence == Confidence::Confirmed));
        assert_eq!(outcome.exit_code, 3);
    }

    #[test]
    fn dream_verdict_advisory_on_review_prompt_only() {
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee plan/0042 for context.\n", false);
        assert!(!outcome.findings.is_empty());
        assert_eq!(outcome.exit_code, 3);
    }

    #[test]
    fn dream_no_false_advisory_verdict() {
        // Clean is 0, not 3; confirmed is 1, not 3.
        let clean = dream_outcome("# M\n\n## L\n\n### X\n\nClean.\n", false);
        assert_ne!(clean.exit_code, 3);
        let confirmed = dream_outcome("# M\n\n## L\n\n### X\n\nSee [[missing-note]].\n", false);
        assert_ne!(confirmed.exit_code, 3);
    }

    #[test]
    fn advisory_verdict_never_saw_confirmed() {
        // Invariant: an advisory verdict implies no confirmed finding.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee call/0017.\n", false);
        assert_eq!(outcome.exit_code, 3);
        assert!(!outcome.findings.iter().any(|f| f.confidence == Confidence::Confirmed));
    }

    #[test]
    fn findings_verdict_implies_confirmed() {
        // Invariant: a findings verdict rests on at least one confirmed finding.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee [[missing-note]].\n", false);
        assert_eq!(outcome.exit_code, 1);
        assert!(outcome.findings.iter().any(|f| f.confidence == Confidence::Confirmed));
    }

    // --- Helper tests (not in the obligations manifest) ---

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
    fn extract_room_refs_survives_multibyte_chars() {
        // Regression: a byte-at-a-time walk used to slice the `str` at `i`,
        // panicking when `i` landed inside a multibyte char (an em-dash).
        let body = "see plan/0074 — the receipt, per call/0018 — reopened";
        let refs = extract_room_refs(body);
        assert_eq!(refs, vec!["plan/0074", "call/0018"]);
        // An em-dash with no following ref must not panic either.
        assert_eq!(extract_room_refs("— just prose —"), Vec::<String>::new());
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

    // --- Manifest tests: detectors (exercises=detect) ---

    #[test]
    fn dream_detects_description_body_drift() {
        let slugs = known(&["alpha"]);
        let inp = input(
            "alpha",
            "not stabilized yet",
            "The API is stabilized as of v0.5.",
            &slugs,
        );
        let fs = detect(&inp);
        assert!(fs.iter().any(|f| f.kind == "description-body-drift"));
    }

    #[test]
    fn dream_clean_on_drift_free_entry() {
        let slugs = known(&["alpha"]);
        let inp = input(
            "alpha",
            "stabilized as of v0.5",
            "The API is stabilized as of v0.5.",
            &slugs,
        );
        let fs = detect(&inp);
        assert!(!fs.iter().any(|f| f.kind == "description-body-drift"));
    }

    #[test]
    fn dream_detects_superseded_unlinked() {
        let slugs = known(&["alpha", "beta"]);
        let mut inp = input("alpha", "older", "Alpha body. See nothing.", &slugs);
        inp.superseded_by = "beta".to_string();
        let fs = detect(&inp);
        assert!(fs.iter().any(|f| f.kind == "superseded-but-unlinked"));
    }

    #[test]
    fn dream_clean_on_linked_supersession() {
        let slugs = known(&["alpha", "beta"]);
        let mut inp = input("alpha", "older", "Older. See [[beta]] for the update.", &slugs);
        inp.superseded_by = "beta".to_string();
        let fs = detect(&inp);
        assert!(!fs.iter().any(|f| f.kind == "superseded-but-unlinked"));
    }

    #[test]
    fn dream_detects_dangling_link() {
        let slugs = known(&["alpha"]);
        let inp = input("alpha", "x", "See [[beta]] which is missing.", &slugs);
        let fs = detect(&inp);
        assert!(fs.iter().any(|f| f.kind == "dangling-link"));
    }

    #[test]
    fn dream_clean_on_resolved_links() {
        let slugs = known(&["alpha", "beta"]);
        let inp = input("alpha", "x", "See [[beta]] which exists.", &slugs);
        let fs = detect(&inp);
        assert!(!fs.iter().any(|f| f.kind == "dangling-link"));
    }

    #[test]
    fn dangling_resolves_against_union_not_one_store() {
        // call/0045: a [[link]] resolves against the union of both stores, so
        // a per-user concept slug satisfies a repo link.
        let slugs = known(&["alpha", "specs-colocate-with-software"]);
        let inp = repo_input("alpha", "x", "See [[specs-colocate-with-software]].", &slugs);
        assert!(!detect(&inp).iter().any(|f| f.kind == "dangling-link"));
        // Unresolved in the union still fires, on either tier.
        let missing = known(&["alpha"]);
        let repo = repo_input("alpha", "x", "See [[specs-colocate-with-software]].", &missing);
        assert!(detect(&repo).iter().any(|f| f.kind == "dangling-link"));
        let per_user = input("alpha", "x", "See [[beta]] which is missing.", &missing);
        assert!(detect(&per_user).iter().any(|f| f.kind == "dangling-link"));
    }

    #[test]
    fn description_body_drift_is_per_user_only_not_repo() {
        // On the repo tier the `### ` heading is fed as the description stand-in,
        // so a "not X" heading over a body discussing X must not false-positive.
        let slugs = known(&["alpha"]);
        let inp = repo_input(
            "alpha",
            "check artifact mismatch is a note not drift",
            "The mismatch is a note, not drift; drift is a separate concern.",
            &slugs,
        );
        let fs = detect(&inp);
        assert!(!fs.iter().any(|f| f.kind == "description-body-drift"));
        // Still fires on the per-user tier.
        let per_user = input("alpha", "not stabilized yet", "The API is stabilized.", &slugs);
        assert!(detect(&per_user).iter().any(|f| f.kind == "description-body-drift"));
    }

    #[test]
    fn dream_detects_room_touching() {
        let slugs = known(&["alpha"]);
        let inp = input("alpha", "x", "Cites call/0017 as live rule.", &slugs);
        let fs = detect(&inp);
        assert!(fs.iter().any(|f| f.kind == "room-touching"));
    }

    #[test]
    fn dream_clean_on_no_room_refs() {
        let slugs = known(&["alpha"]);
        let inp = input("alpha", "x", "No methodology records cited here.", &slugs);
        let fs = detect(&inp);
        assert!(!fs.iter().any(|f| f.kind == "room-touching"));
    }

    #[test]
    fn repo_store_finding_routes_to_append() {
        // Invariant RepoFindingsNeverEdit: a repo-store finding never routes
        // to edit. Encoded by `route_for(StoreLoc::Repo) == Route::Append`.
        let slugs = known(&["alpha"]);
        let mut inp = input("alpha", "x", "Cites call/0017.", &slugs);
        inp.store = StoreLoc::Repo;
        let fs = detect(&inp);
        for f in &fs {
            assert_ne!(f.route, Route::Edit, "repo finding routed to edit: {:?}", f);
            assert_eq!(f.route, Route::Append);
        }
    }

    #[test]
    fn per_user_finding_suggestion_carries_the_edit_imperative() {
        // W1 (plan/0073 fen-acceptance): the suggestion, not the `route=` token,
        // is what a 4B reads. A per-user finding's imperative must say EDIT in
        // place and forbid the repo append, or the model appends anyway.
        let slugs = known(&["alpha"]);
        let mut inp = input("alpha", "x", "See [[beta]] which is missing.", &slugs);
        inp.store = StoreLoc::PerUser;
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "dangling-link")
            .expect("dangling-link finding");
        assert!(f.suggestion.starts_with("Edit"), "not an edit imperative: {}", f.suggestion);
        assert!(f.suggestion.contains("in place"), "missing in-place cue: {}", f.suggestion);
        assert!(
            f.suggestion.contains("Do not append"),
            "missing anti-append tail: {}",
            f.suggestion
        );
    }

    #[test]
    fn repo_finding_suggestion_carries_the_append_imperative() {
        // The mirror of the above: the repo route (Append) imperative must say
        // APPEND a new entry and forbid the in-place edit. Tested on the routing
        // frame directly, since the single-entry detectors that carry it are
        // per-user-only (dangling-link, description-body-drift) and room-touching
        // carries its own bespoke MADR imperative.
        let s = suggestion_for(Route::Append, "alpha", "resolve the thing");
        assert!(s.starts_with("Append"), "not an append imperative: {s}");
        assert!(s.contains("Do not edit"), "missing anti-edit tail: {s}");
    }

    #[test]
    fn room_touching_suggestion_routes_to_madr_not_a_memory_write() {
        // Room-touching is neither an edit nor an append: the imperative names
        // the record and the MADR action, and forbids both memory writes.
        let slugs = known(&["alpha"]);
        let inp = input("alpha", "x", "Cites call/0017 as live rule.", &slugs);
        let f = detect(&inp)
            .into_iter()
            .find(|f| f.kind == "room-touching")
            .expect("room-touching finding");
        assert!(f.suggestion.contains("call/0017"), "record not named: {}", f.suggestion);
        assert!(f.suggestion.contains("Status: superseded"), "no MADR action: {}", f.suggestion);
        assert!(
            f.suggestion.contains("Do not edit the memory entry"),
            "missing anti-memory-write tail: {}",
            f.suggestion
        );
    }

    // --- Manifest tests: verdict lifecycle (exercises=dream) ---
    // These drive `dream::run_dream` over a fixture and assert the exit code
    // (the verdict) matches the spec's transition graph.

    fn dream_outcome(memory_md: &str, fix: bool) -> DreamOutcome {
        let dir = tmp_fixture("lifecycle", memory_md);
        let mut args = vec![dir.to_string_lossy().to_string()];
        if fix {
            args.push("--fix".to_string());
        }
        // `dream::run_dream` is the testable core; the public `dream` wrapper
        // adds process::exit. Reference: the dream module's run_dream.
        let outcome = run_dream(&args);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    #[test]
    fn dream_transition_dreaming_to_clean() {
        // A clean fixture (no room refs, no links): dreaming -> clean (exit 0).
        let outcome = dream_outcome("# M\n\n## L\n\n### Clean\n\nNothing to flag.\n", false);
        assert_eq!(outcome.exit_code, 0);
    }

    #[test]
    fn dream_transition_dreaming_to_findings() {
        // A confirmed finding (a dangling link with no tier marker):
        // dreaming -> findings (exit 1).
        let outcome = dream_outcome("# M\n\n## L\n\n### Cites\n\nSee [[missing-note]].\n", false);
        assert_eq!(outcome.exit_code, 1);
    }

    #[test]
    fn dream_transition_dreaming_to_advisory() {
        // Review prompts alone (a room-touching citation): dreaming ->
        // advisory (exit 3, the call/0045 split).
        let outcome = dream_outcome("# M\n\n## L\n\n### Cites\n\nSee call/0017.\n", false);
        assert_eq!(outcome.exit_code, 3);
    }

    #[test]
    fn dream_transition_dreaming_to_error() {
        // --fix on a fixture with a repo finding: dreaming -> error (exit 2).
        let outcome = dream_outcome("# M\n\n## L\n\n### Cites\n\nSee call/0017.\n", true);
        assert_eq!(outcome.exit_code, 2);
    }

    #[test]
    fn dream_start_inits_latch_and_status() {
        // run_dream starts each call with an empty findings vector (the latch
        // is fresh per run; no stale state leaks across calls).
        let outcome = dream_outcome("# M\n\n## L\n\n### Clean\n\nNothing.\n", false);
        assert!(outcome.findings.is_empty(), "findings should be empty on a clean run");
        // A second run on a dirty fixture should find exactly the new findings,
        // not any leftover from the prior clean run.
        let outcome2 = dream_outcome("# M\n\n## L\n\n### Dirty\n\nSee plan/0042.\n", false);
        assert!(!outcome2.findings.is_empty());
    }

    #[test]
    fn dream_record_finding_sets_latch() {
        // Any finding fires the latch: exit 1 iff findings is non-empty.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee [[missing-note]].\n", false);
        assert_eq!(outcome.exit_code, 1);
        assert!(!outcome.findings.is_empty());
    }

    #[test]
    fn dream_record_finding_guarded_on_dreaming() {
        // The latch is per-run (single-shot model); a finding recorded in one
        // run does not leak into the next run's verdict. Run two audits back
        // to back and confirm the second's verdict matches its own findings.
        let dirty = dream_outcome("# M\n\n## L\n\n### X\n\nSee [[missing-note]].\n", false);
        assert_eq!(dirty.exit_code, 1);
        let clean = dream_outcome("# M\n\n## L\n\n### Y\n\nClean entry.\n", false);
        assert_eq!(clean.exit_code, 0, "clean run after a dirty run must still be clean");
    }

    #[test]
    fn dream_verdict_findings_on_any_finding() {
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee [[missing-note]].\n", false);
        assert_eq!(outcome.exit_code, 1);
    }

    #[test]
    fn dream_no_false_findings_verdict() {
        // A clean fixture never yields exit 1.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nClean.\n", false);
        assert_ne!(outcome.exit_code, 1);
    }

    #[test]
    fn dream_verdict_clean_on_no_finding() {
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nClean.\n", false);
        assert_eq!(outcome.exit_code, 0);
    }

    #[test]
    fn dream_no_false_clean_verdict() {
        // A fixture with a finding never yields exit 0.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee [[missing-note]].\n", false);
        assert_ne!(outcome.exit_code, 0);
    }

    #[test]
    fn dream_verdict_error_on_fail_signal() {
        // --fix on a repo-store finding is the fail signal: exit 2.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nSee call/0017.\n", true);
        assert_eq!(outcome.exit_code, 2);
    }

    #[test]
    fn dream_no_false_error_verdict() {
        // --fix on a clean fixture (no repo findings) does not yield exit 2.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nClean.\n", true);
        assert_ne!(outcome.exit_code, 2);
    }

    #[test]
    fn clean_verdict_implies_no_finding() {
        // Invariant VerdictMatchesFindings: exit 0 implies findings is empty.
        let outcome = dream_outcome("# M\n\n## L\n\n### X\n\nClean.\n", false);
        if outcome.exit_code == 0 {
            assert!(outcome.findings.is_empty());
        }
    }

    // --- Manifest tests: the three #implement-remaining detectors ---

    #[test]
    fn dream_detects_stale_state_over_lore() {
        // A State entry whose body reads done and carries a measured value.
        let slugs = known(&["snapshot"]);
        let mut inp = input(
            "snapshot",
            "GPU parity status",
            "Single-GPU parity is done; the dual-GPU measurement was 42ms.",
            &slugs,
        );
        inp.entry_type = EntryType::State;
        let fs = detect(&inp);
        assert!(fs.iter().any(|f| f.kind == "stale-state-over-lore"));
    }

    #[test]
    fn dream_clean_on_current_state() {
        // A State entry that is done but has no measured lore: not flagged.
        let slugs = known(&["snapshot"]);
        let mut inp = input(
            "snapshot",
            "Status",
            "This work is done and shipped.",
            &slugs,
        );
        inp.entry_type = EntryType::State;
        let fs = detect(&inp);
        assert!(!fs.iter().any(|f| f.kind == "stale-state-over-lore"));
    }

    #[test]
    fn dream_detects_workaround_vs_plan() {
        // A Workaround entry and a Fact entry in the same store, sharing a
        // key term, neither cross-linking the other.
        let slugs = known(&["workaround", "plan"]);
        let mut w = input(
            "workaround",
            "k=0 kernel fix",
            "The k=0_kernel needs the AR16 model applied at boot.",
            &slugs,
        );
        w.entry_type = EntryType::Workaround;
        let mut p = input(
            "plan",
            "Base locked",
            "The base is locked to F16-ssm_out as agreed.",
            &slugs,
        );
        p.entry_type = EntryType::Fact;
        // No shared key term yet; force one by adjusting the plan body.
        p.body = "The base is locked to k=0_kernel as agreed.".to_string();
        let entries = vec![w.clone(), p];
        let fs = detect_cross(&entries);
        assert!(fs.iter().any(|f| f.kind == "workaround-vs-plan" && f.entry_slug == "workaround"));
    }

    #[test]
    fn dream_clean_on_workaround_linked_to_plan() {
        // Same as above but the workaround entry cross-links the plan: not flagged.
        let slugs = known(&["workaround", "plan"]);
        let mut w = input(
            "workaround",
            "k=0 kernel fix",
            "The k=0_kernel needs AR16; see [[plan]] for the standing decision.",
            &slugs,
        );
        w.entry_type = EntryType::Workaround;
        let mut p = input(
            "plan",
            "Base locked",
            "The base is locked to k=0_kernel as agreed.",
            &slugs,
        );
        p.entry_type = EntryType::Fact;
        let entries = vec![w, p];
        let fs = detect_cross(&entries);
        assert!(!fs.iter().any(|f| f.kind == "workaround-vs-plan"));
    }

    #[test]
    fn dream_detects_append_only_violation_in_git_history() {
        // Build a real git repo, commit MEMORY.md, then edit a section's body
        // and commit again. The detector should find the edited section.
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!(
            "dream-append-only-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| {
            let _ = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
        };
        g(&["init", "-q"]);
        g(&["config", "user.name", "test"]);
        g(&["config", "user.email", "test@example.com"]);
        fs::write(
            dir.join("MEMORY.md"),
            "# MEMORY.md\n\n### Original\n\nFirst body line.\n",
        )
        .unwrap();
        g(&["add", "MEMORY.md"]);
        g(&["commit", "-q", "-m", "first"]);
        // Edit the section's body in place (no new section heading).
        fs::write(
            dir.join("MEMORY.md"),
            "# MEMORY.md\n\n### Original\n\nEdited body line.\n",
        )
        .unwrap();
        g(&["add", "MEMORY.md"]);
        g(&["commit", "-q", "-m", "second"]);
        let path = dir.join("MEMORY.md");
        let fs_out = detect_append_only_violations(&path).0;
        assert!(
            fs_out.iter().any(|f| f.kind == "append-only-violation"),
            "expected an append-only-violation finding, got: {fs_out:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dream_clean_on_clean_repo_store() {
        // A git history with only additions (new sections, no body edits to
        // existing ones) yields no append-only-violation findings.
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!(
            "dream-clean-repo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| {
            let _ = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
        };
        g(&["init", "-q"]);
        g(&["config", "user.name", "test"]);
        g(&["config", "user.email", "test@example.com"]);
        fs::write(dir.join("MEMORY.md"), "# MEMORY.md\n\n### First\n\nBody.\n").unwrap();
        g(&["add", "MEMORY.md"]);
        g(&["commit", "-q", "-m", "first"]);
        // Append a new section (allowed).
        fs::write(
            dir.join("MEMORY.md"),
            "# MEMORY.md\n\n### First\n\nBody.\n\n### Second\n\nAppended body.\n",
        )
        .unwrap();
        g(&["add", "MEMORY.md"]);
        g(&["commit", "-q", "-m", "second"]);
        let path = dir.join("MEMORY.md");
        let fs_out = detect_append_only_violations(&path).0;
        assert!(
            !fs_out.iter().any(|f| f.kind == "append-only-violation"),
            "expected no append-only-violation findings on a clean-append history, got: {fs_out:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dream_append_only_violation_finding_created() {
        // rule-entity-creation obligation: the detector emits a Finding struct
        // with the right kind, store, and route when git history shows an edit.
        // Covered by dream_detects_append_only_violation_in_git_history; this
        // test asserts the Finding shape explicitly.
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!(
            "dream-append-shape-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| {
            let _ = Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
        };
        g(&["init", "-q"]);
        g(&["config", "user.name", "test"]);
        g(&["config", "user.email", "test@example.com"]);
        fs::write(dir.join("MEMORY.md"), "# M\n\n### X\n\nOriginal.\n").unwrap();
        g(&["add", "MEMORY.md"]);
        g(&["commit", "-q", "-m", "first"]);
        fs::write(dir.join("MEMORY.md"), "# M\n\n### X\n\nEdited.\n").unwrap();
        g(&["add", "MEMORY.md"]);
        g(&["commit", "-q", "-m", "second"]);
        let path = dir.join("MEMORY.md");
        let fs_out = detect_append_only_violations(&path).0;
        let f = fs_out
            .iter()
            .find(|f| f.kind == "append-only-violation")
            .expect("append-only-violation finding not emitted");
        assert_eq!(f.store, StoreLoc::Repo);
        assert_eq!(f.route, Route::Append);
        let _ = fs::remove_dir_all(&dir);
    }
}

// === Kani proofs (plan/0073 #write-tests): the load-bearing pure-function
// invariants. Reference: plan/0023 verification ladder, host-prove
// `kani-conformance` lane. The `kani` feature is set by `cargo kani`; the
// harnesses are inert under `cargo test` and `cargo build`.
// Kani dispositions in the obligations manifest point at these harnesses. ===

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// RepoFindingsNeverEdit: `route_for(StoreLoc::Repo)` always returns
    /// `Route::Append`, never `Route::Edit`. The detect() engine's per-detector
    /// else-branch hardcodes `Route::Append` for repo entries (verifiable by
    /// reading the source); this harness proves the routing function itself
    /// never returns Edit for a repo store, which is the load-bearing property
    /// the unit test `repo_store_finding_routes_to_append` exercises
    /// behaviourally.
    #[kani::proof]
    fn route_for_repo_always_appends() {
        assert!(DetectorInput::route_for(StoreLoc::Repo) == Route::Append);
        assert!(DetectorInput::route_for(StoreLoc::PerUser) == Route::Edit);
    }
}
