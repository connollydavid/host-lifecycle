//! Resolving a register reference, and sweeping a tree for the references its
//! published site cannot render (plan/0077, host-lifecycle#17).
//!
//! The methodology makes a number an identity: a bare number at the plan root
//! names a milestone, a numbered file under `call/` names a decision, and an
//! issue number names work in a repository. Every room and every document refers
//! to those numbers, and nothing resolved them, so a reference read like a link
//! and behaved like text.
//!
//! `resolve` takes one reference and prints where it points: the path by
//! default, a markdown link with `--markdown`, the full forge URL with `--url`,
//! with any `#anchor` carried through so a task node resolves to its heading.
//!
//! `refs --check` sweeps the authored markdown and reports what a reader cannot
//! follow: a register reference that points at nothing (a dead pointer, which
//! gates) and an issue number written bare (legibility debt, which advises).
//! The record layer is excluded by the project's own exclusion list, because an
//! append-only log is never rewritten to satisfy a checker.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

/// The rooms whose numbered entries a reference can name.
const ROOMS: [&str; 2] = ["plan", "call"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Register,
    Issue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Resolved,
    /// The room was searched and holds no such entry. In a software repository
    /// this is the ordinary case: the number belongs to the governing host, and
    /// the reference carries no repository to look in.
    UnresolvedHere,
    Malformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    Path,
    MarkdownLink,
    FullUrl,
}

/// A dead pointer gates; legibility debt advises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    DeadPointer,
    Unrendered,
}

/// One reference as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub kind: RefKind,
    /// `plan` or `call` for a register reference.
    pub room: String,
    /// The repository an issue names, as written: `owner/repo` or a bare
    /// component name. Empty for a register reference, and empty for a bare `#N`,
    /// which is the case this tool refuses to guess at: in this very tree most
    /// bare numbers name host-lifecycle issues while the origin remote is the
    /// host, so a guessed link would be confidently wrong.
    pub repo: String,
    pub number: String,
    pub anchor: Option<String>,
}

/// Parse a reference as written. `plan/0074`, `call/0045`, `plan/0074#write-spec`
/// and `#17` are references; anything else is not.
pub fn parse_reference(text: &str) -> Option<Reference> {
    let t = text.trim();
    // A register reference first: `plan/0074#write-spec` ends in an anchor, not in
    // an issue number, and reading it as an issue would reject it outright.
    if let Some((room, rest)) = t.split_once('/') {
        if ROOMS.contains(&room) {
            let (number, anchor) = match rest.split_once('#') {
                Some((n, a)) => (n, Some(a.to_string())),
                None => (rest, None),
            };
            // Four digits is the register's shape; `plan/074` and `plan/00741` are
            // not references, so a typo reads as malformed rather than resolving
            // somewhere else.
            if number.len() == 4 && number.chars().all(|c| c.is_ascii_digit()) {
                return Some(Reference {
                    kind: RefKind::Register,
                    room: room.to_string(),
                    repo: String::new(),
                    number: number.to_string(),
                    anchor,
                });
            }
            return None;
        }
    }
    // An issue: `owner/repo#N`, `component#N`, or a bare `#N` whose repository is
    // not written (which `emit` refuses to guess at).
    let (repo, n) = t.rsplit_once('#')?;
    if n.is_empty() || n.len() > 6 || !n.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if repo.contains(char::is_whitespace) || repo.matches('/').count() > 1 {
        return None;
    }
    if !repo.is_empty() && !repo.starts_with(|c: char| c.is_alphanumeric()) {
        return None;
    }
    Some(Reference {
        kind: RefKind::Issue,
        room: String::new(),
        repo: repo.to_string(),
        number: n.to_string(),
        anchor: None,
    })
}

/// Every entry in the room carrying this number. More than one is an ambiguity
/// the caller must refuse rather than resolve: taking the alphabetically first
/// handed out a confident path to an abandoned draft.
pub fn entry_matches(root: &Path, reference: &Reference) -> Vec<PathBuf> {
    if reference.kind != RefKind::Register {
        return Vec::new();
    }
    let Ok(dir) = fs::read_dir(root.join(&reference.room)) else {
        return Vec::new();
    };
    let mut matches: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.strip_prefix(&reference.number).is_some_and(|r| r.starts_with('-')))
        })
        .collect();
    matches.sort();
    matches
}

pub fn entry_path(root: &Path, reference: &Reference) -> Option<PathBuf> {
    let matches = entry_matches(root, reference);
    if matches.len() != 1 {
        return None;
    }
    let hit = matches.into_iter().next()?;
    if !hit.is_dir() {
        return hit.exists().then_some(hit);
    }
    // A milestone is a directory whose README is the page. Where there is no
    // README the directory itself IS the record, and reporting it as naming no
    // entry was a gating verdict about a milestone that is plainly there.
    let readme = hit.join("README.md");
    if readme.exists() {
        Some(readme)
    } else {
        hit.is_dir().then_some(hit)
    }
}

/// The repository's forge coordinates, from the origin remote: `owner/repo`.
/// `None` in a repository with no remote, which is why a URL can fail to build
/// while a path cannot.
/// The default branch of the origin remote, for a blob URL. `main` only as a
/// last resort: a repository on `master` was handed a 404.
pub fn default_branch(root: &Path) -> String {
    crate::git_out(root, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .and_then(|r| r.trim().rsplit('/').next().map(str::to_string))
        .or_else(|| crate::git_out(root, &["symbolic-ref", "--short", "HEAD"]).map(|r| r.trim().to_string()))
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".to_string())
}

pub fn origin_slug(root: &Path) -> Option<String> {
    let url = crate::git_out(root, &["remote", "get-url", "origin"])?;
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    // Matched case-insensitively: `https://GitHub.com/o/r` is the same forge, and
    // refusing it reported "no github origin remote" about a GitHub repository.
    let lower = url.to_ascii_lowercase();
    let at = lower.rfind("github.com")?;
    let rest = url[at + "github.com".len()..].trim_start_matches([':', '/']);
    (rest.matches('/').count() == 1).then(|| rest.to_string())
}

/// The owner this tree RECORDS for a component, from the software recipe or the
/// submodule list. A component's owner was guessed from the local origin, so
/// `allium#12` resolved to this project's namespace while `.gitmodules` recorded
/// another, and every URL in a fork retargeted to the contributor.
fn recorded_owner(root: &Path, component: &str) -> Option<String> {
    let mut sources = Vec::new();
    if let Ok(text) = fs::read_to_string(root.join(".host-software")) {
        sources.push(text);
    }
    if let Ok(text) = fs::read_to_string(root.join(".gitmodules")) {
        sources.push(text);
    }
    for text in sources {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            if key.trim() != "url" {
                continue;
            }
            let url = value.trim().trim_end_matches('/').trim_end_matches(".git");
            let Some((owner, repo)) = url.rsplit_once('/').and_then(|(o, r)| {
                o.rsplit_once(['/', ':']).map(|(_, owner)| (owner.to_string(), r.to_string()))
            }) else {
                continue;
            };
            if repo.eq_ignore_ascii_case(component) {
                return Some(owner);
            }
        }
    }
    None
}

/// The forge host the origin names, for a reference that already carries its
/// repository. A qualified `o/r#17` in a GitLab-hosted project meant GitLab, and
/// building a github.com URL for it produced a link to an unrelated repository.
fn origin_host(root: &Path) -> Option<String> {
    let url = crate::git_out(root, &["remote", "get-url", "origin"])?;
    let url = url.trim();
    let after_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let after_user = after_scheme.rsplit_once('@').map(|(_, r)| r).unwrap_or(after_scheme);
    let host = after_user.split([':', '/']).next()?;
    (!host.is_empty() && host.contains('.')).then(|| host.to_ascii_lowercase())
}

/// The anchor GitHub actually generates for a heading carrying an explicit
/// `{#id}`. The site renders the braces as literal text and slugifies the whole
/// line, so emitting the explicit id produced a fragment matching no element and
/// dropped the reader at the top of a long document.
fn github_anchor(entry: &Path, anchor: &str) -> Option<String> {
    let text = fs::read_to_string(entry).ok()?;
    let marker = format!("{{#{anchor}}}");
    let heading = text
        .lines()
        .find(|l| l.trim_start().starts_with('#') && l.contains(&marker))?;
    let title = heading.trim_start().trim_start_matches('#').trim();
    let slug: String = title
        .to_ascii_lowercase()
        .chars()
        .filter_map(|c| match c {
            c if c.is_alphanumeric() => Some(c),
            ' ' | '-' | '_' => Some('-'),
            _ => None,
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    (!slug.is_empty()).then_some(slug)
}

/// Whether this repository owns the room a register reference names. A software
/// repository has no `plan/` or `call/` room: the numbers in its documents belong
/// to its governing host, and the reference carries no repository to look in. A
/// sweep that called those dead would turn every software repository red for
/// citing the decisions that govern it.
pub fn owns_room(root: &Path, reference: &Reference) -> bool {
    reference.kind == RefKind::Register && root.join(&reference.room).is_dir()
}

/// What resolving established, without emitting anything: the sweep asks this of
/// every reference it finds, and the CLI asks it of the one it was given.
pub fn resolution_of(root: &Path, reference: &Reference) -> Resolution {
    match reference.kind {
        RefKind::Register => match entry_path(root, reference) {
            Some(_) => Resolution::Resolved,
            None => Resolution::UnresolvedHere,
        },
        // An issue resolves to a forge URL, and only where the remote says which
        // forge and which repository.
        // An issue resolves when it names its repository (or can take an owner
        // from the origin); a bare number names nothing this tool can resolve.
        RefKind::Issue => {
            let named = reference.repo.contains('/');
            let owned = !reference.repo.is_empty() && origin_slug(root).is_some();
            if named || owned {
                Resolution::Resolved
            } else {
                Resolution::UnresolvedHere
            }
        }
    }
}

/// The same question asked of a reference as WRITTEN: text that is not a
/// reference at all is malformed, which is a different answer from a reference
/// this repository cannot resolve.
pub fn resolution(root: &Path, text: &str) -> Resolution {
    match parse_reference(text) {
        Some(reference) => resolution_of(root, &reference),
        None => Resolution::Malformed,
    }
}

/// What a resolution prints, in the form asked for.
pub fn emit(root: &Path, reference: &Reference, emission: Emission) -> Result<String, String> {
    let anchor = reference.anchor.as_ref().map(|a| format!("#{a}")).unwrap_or_default();
    match reference.kind {
        RefKind::Issue => {
            // An issue lives in the forge, never on disk: that is true whatever the
            // remote says, so it is answered before the remote is read.
            if emission == Emission::Path {
                return Err(format!("#{} names work in a forge, not a path", reference.number));
            }
            // A bare `#N` names no repository, and guessing the local origin is how
            // a link ends up pointing at the wrong tracker: in this host most bare
            // numbers name host-lifecycle issues while the origin is the host.
            if reference.repo.is_empty() {
                return Err(format!(
                    "#{} names no repository. Write it as owner/repo#{} (or component#{}) so the link is unambiguous",
                    reference.number, reference.number, reference.number
                ));
            }
            let slug = if reference.repo.contains('/') {
                reference.repo.clone()
            } else {
                // A bare component name takes the owner this tree RECORDS for it,
                // and only falls back to the origin's owner when nothing records
                // one. The recipe and the submodule list already name the owner,
                // so guessing from origin retargeted a component held elsewhere
                // and retargeted everything in a fork.
                let owner = recorded_owner(root, &reference.repo)
                    .or_else(|| origin_slug(root).and_then(|s| s.split_once('/').map(|(o, _)| o.to_string())))
                    .ok_or_else(|| {
                        format!(
                            "nothing here records an owner for `{}`, and there is no github origin to take one from; write owner/repo#{}",
                            reference.repo, reference.number
                        )
                    })?;
                format!("{owner}/{}", reference.repo)
            };
            // The forge follows the origin. A qualified reference in a project
            // hosted elsewhere means that forge, and a github.com URL for it
            // pointed at an unrelated repository that may well exist.
            let host = origin_host(root).unwrap_or_else(|| "github.com".to_string());
            match emission {
                Emission::FullUrl => Ok(format!("https://{host}/{slug}/issues/{}", reference.number)),
                Emission::MarkdownLink => Ok(format!(
                    "[{slug}#{}](https://{host}/{slug}/issues/{})",
                    reference.number, reference.number
                )),
                Emission::Path => unreachable!("answered above, before the remote is read"),
            }
        }
        RefKind::Register => {
            let rel = entry_path(root, reference).ok_or_else(|| unresolved_reason(root, reference))?;
            let rel = rel.strip_prefix(root).unwrap_or(&rel).to_string_lossy().replace('\\', "/");
            match emission {
                Emission::Path => Ok(format!("{rel}{anchor}")),
                Emission::MarkdownLink => Ok(format!(
                    "[{}/{}{anchor}]({rel}{anchor})",
                    reference.room, reference.number
                )),
                Emission::FullUrl => {
                    let slug = origin_slug(root)
                        .ok_or_else(|| "no github origin remote, so a URL cannot be built".to_string())?;
                    // The forge slugifies the whole heading line, braces and all,
                    // so the explicit id is carried into the fragment only after
                    // it has been translated into the anchor the forge holds.
                    let fragment = match (&reference.anchor, entry_path(root, reference)) {
                        (Some(a), Some(entry)) => match github_anchor(&entry, a) {
                            Some(slugged) => format!("#{slugged}"),
                            // Better to land at the top of the right document than
                            // at a fragment the forge does not have.
                            None => String::new(),
                        },
                        _ => String::new(),
                    };
                    Ok(format!(
                        "https://github.com/{slug}/blob/{}/{rel}{fragment}",
                        default_branch(root)
                    ))
                }
            }
        }
    }
}

/// Why a register reference did not resolve, in the terms of what the tree
/// actually holds. One sentence for three different situations was a diagnosis
/// that was wrong twice: a repository owning the room was told the number
/// belonged to its governing host, and an ambiguity was reported as an absence.
fn unresolved_reason(root: &Path, reference: &Reference) -> String {
    let matches = entry_matches(root, reference);
    if matches.len() > 1 {
        let names: Vec<String> = matches
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
            .collect();
        return format!(
            "ambiguous: {}/{} names {} entries in {}/ ({}). Rename or remove one; a reference that resolves two ways resolves to neither",
            reference.room,
            reference.number,
            names.len(),
            reference.room,
            names.join(", ")
        );
    }
    if !root.join(&reference.room).is_dir() {
        return format!(
            "unresolved here: this repository holds no {}/ room, so {}/{} names a register of its governing host and cannot be resolved from here",
            reference.room, reference.room, reference.number
        );
    }
    format!(
        "unresolved here: {}/ exists and holds no entry numbered {}",
        reference.room, reference.number
    )
}

/// One reported reference.
#[derive(Debug, Clone)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub text: String,
    pub weight: Weight,
}

/// The references a line carries, with the document facts the sweep needs. A
/// reference already inside a markdown link renders; one inside fenced code is an
/// example rather than a reference.
fn scan_line(line: &str) -> Vec<(Reference, bool)> {
    let mut out = Vec::new();
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        // A register reference: `plan/NNNN` or `call/NNNN`, optionally anchored.
        for room in ROOMS {
            let room_chars: Vec<char> = room.chars().collect();
            if i + room_chars.len() + 5 <= bytes.len()
                && bytes[i..i + room_chars.len()] == room_chars[..]
                && bytes[i + room_chars.len()] == '/'
                && (i == 0 || !bytes[i - 1].is_alphanumeric())
            {
                let start = i + room_chars.len() + 1;
                let digits: String = bytes[start..].iter().take_while(|c| c.is_ascii_digit()).collect();
                if digits.len() == 4 {
                    let mut end = start + 4;
                    let mut anchor = None;
                    if end < bytes.len() && bytes[end] == '#' {
                        let a: String = bytes[end + 1..]
                            .iter()
                            .take_while(|c| c.is_alphanumeric() || **c == '-' || **c == '_')
                            .collect();
                        if !a.is_empty() {
                            end += 1 + a.chars().count();
                            anchor = Some(a);
                        }
                    }
                    out.push((
                        Reference {
                            kind: RefKind::Register,
                            room: room.to_string(),
                            repo: String::new(),
                            number: digits,
                            anchor,
                        },
                        enclosing_link(&bytes, i, end),
                    ));
                    i = end;
                }
            }
        }
        if i >= bytes.len() {
            break;
        }
        // An issue reference: `#N`, not preceded by a word character (which would
        // make it an anchor or a fragment) and not inside a link.
        if bytes[i] == '#' {
            // What precedes the `#`: a repository name makes this a complete
            // reference, a word character makes it a fragment rather than one.
            let mut r = i;
            while r > 0 && (bytes[r - 1].is_alphanumeric() || bytes[r - 1] == '-' || bytes[r - 1] == '/' || bytes[r - 1] == '_' || bytes[r - 1] == '.') {
                r -= 1;
            }
            let written: String = bytes[r..i].iter().collect();
            let digits: String = bytes[i + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
            // A CSS colour is not an issue: `#0077cc` and `#123456` are six hex
            // digits, and an issue number followed by a hex letter is not a number.
            let hex_tail = bytes
                .get(i + 1 + digits.chars().count())
                .is_some_and(|c| c.is_ascii_hexdigit() && !c.is_ascii_digit());
            // A URL fragment is not an issue: what precedes the `#` in
            // `https://example.com/spec/page#2024` is a path, and the same shape
            // check `parse_reference` applies keeps it out of the corpus. A bare
            // shorthand colour (`#123`, `#1234`) is not one either.
            // A bare six-digit number is a colour rather than an issue; three and
            // four digit shorthand colours are left in, because `#123` outside
            // code is far likelier to be issue 123, and dropping it would lose
            // real references to catch a shape this corpus does not contain.
            // A repository name is `owner/repo` or a bare component, and both
            // halves have to be there: `#41/#50` walked back over the slash and
            // read `41/` as a repository, so a pair of review-finding codes was
            // counted as an issue reference.
            let repo_shaped = written.is_empty()
                || (written.starts_with(|c: char| c.is_alphanumeric())
                    && match written.split_once('/') {
                        Some((owner, repo)) => !owner.is_empty() && !repo.is_empty() && !repo.contains('/'),
                        None => true,
                    });
            if !digits.is_empty() && digits.len() <= 6 && !hex_tail && repo_shaped && !(digits.len() == 6 && written.is_empty()) {
                let end = i + 1 + digits.chars().count();
                out.push((
                    Reference {
                        kind: RefKind::Issue,
                        room: String::new(),
                        repo: written,
                        number: digits,
                        anchor: None,
                    },
                    enclosing_link(&bytes, i, end),
                ));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The document's prose, with everything that quotes rather than refers removed:
/// fenced blocks (including one inside a blockquote), indented code blocks, HTML
/// comments, and inline code spans. Each entry is `(line number, prose)`, and a
/// skipped line simply does not appear.
///
/// Backtick parity per line was the earlier reading, and it was wrong in both
/// directions: a code span wrapped across a line break left odd parity and hid
/// every reference after it, while a nested double-backtick span reported one it
/// should have quoted. Runs are matched here the way a markdown reader matches
/// them, so an unmatched backtick is literal and quotes nothing.
fn prose_of(text: &str) -> Vec<(usize, String)> {
    let mut kept: Vec<(usize, String)> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut in_comment = false;
    let mut in_indented = false;
    let mut prev_blank = true;
    for (n, raw) in text.lines().enumerate() {
        // A blockquote marker is presentation: `> ```` opens a fence exactly as a
        // bare one does, and reading the quoted block as prose produced a dead
        // pointer finding about an example. Only the markers come off — the
        // indentation has to survive, because four spaces after a blank line is
        // what makes the next line code.
        let mut body = raw;
        while let Some(rest) = body.trim_start_matches(' ').strip_prefix('>') {
            body = rest.strip_prefix(' ').unwrap_or(rest);
        }
        let t = body.trim_start();
        if in_comment {
            if let Some(p) = t.find("-->") {
                in_comment = false;
                kept.push((n + 1, t[p + 3..].to_string()));
            }
            continue;
        }
        if let Some((marker, len)) = fence {
            let run = t.chars().take_while(|c| *c == marker).count();
            if run >= len && t[run..].trim().is_empty() {
                fence = None;
            }
            continue;
        }
        let run = t.chars().take_while(|c| *c == '`').count();
        let tilde = t.chars().take_while(|c| *c == '~').count();
        if run >= 3 {
            fence = Some(('`', run));
            continue;
        }
        if tilde >= 3 {
            fence = Some(('~', tilde));
            continue;
        }
        let blank = t.is_empty();
        // An indented code block: four spaces (or a tab) opening after a blank
        // line, and every indented line after it. The blank-line condition is
        // what keeps an indented continuation inside a list from being read as
        // code and dropped from coverage.
        let indented = body.starts_with("    ") || body.starts_with('\t');
        if !blank && indented && (prev_blank || in_indented) {
            in_indented = true;
            prev_blank = false;
            continue;
        }
        if !blank && !indented {
            in_indented = false;
        }
        prev_blank = blank;
        if let Some(p) = t.find("<!--") {
            match t[p..].find("-->") {
                Some(e) => kept.push((n + 1, format!("{}{}", &t[..p], &t[p + e + 3..]))),
                None => {
                    in_comment = true;
                    kept.push((n + 1, t[..p].to_string()));
                }
            }
            continue;
        }
        kept.push((n + 1, body.to_string()));
    }
    // Code spans are matched across the joined prose, because a span may wrap a
    // line break. Masking with spaces keeps every other column where it was.
    let joined: String = kept.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n");
    let masked = mask_code_spans(&joined);
    kept.iter()
        .map(|(n, _)| *n)
        .zip(masked.split('\n').map(String::from))
        .collect()
}

/// Blank out every inline code span, matching a run of backticks with the next
/// run of the SAME length, exactly as a markdown reader does. An opening run with
/// no match is literal text and masks nothing.
fn mask_code_spans(buf: &str) -> String {
    let chars: Vec<char> = buf.chars().collect();
    let mut out = chars.clone();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let start = i;
        let mut open = 0usize;
        while i < chars.len() && chars[i] == '`' {
            open += 1;
            i += 1;
        }
        let mut j = i;
        let mut close = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let run_start = j;
                let mut run = 0usize;
                while j < chars.len() && chars[j] == '`' {
                    run += 1;
                    j += 1;
                }
                if run == open {
                    close = Some((run_start, j));
                    break;
                }
            } else {
                j += 1;
            }
        }
        if let Some((_, end)) = close {
            for c in out.iter_mut().take(end).skip(start) {
                if *c != '\n' {
                    *c = ' ';
                }
            }
            i = end;
        }
    }
    out.into_iter().collect()
}

/// Whether the span sits inside a markdown link: either the label or the target
/// of a `[...](...)`. Such a reference renders, so it is not debt.
fn enclosing_link(line: &[char], start: usize, end: usize) -> bool {
    let before: String = line[..start].iter().collect();
    let after: String = line[end..].iter().collect();
    let in_label = before.rfind('[').is_some_and(|b| {
        before[b..].find(']').is_none() && after.find(']').is_some_and(|c| after[c..].starts_with("]("))
    });
    let in_target = before.rfind("](").is_some_and(|b| before[b..].find(')').is_none()) && after.contains(')');
    // A bare autolink (`<https://…/17>`) also renders, and so does a reference
    // written inside inline code, which is quoting rather than referring.
    in_label || in_target
}

/// Sweep one document. Fenced code is skipped: a fenced `#3` is an example, which
/// is how the tell gate reads its own fixtures.
pub fn scan_document(text: &str, file: &str, root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    // One reference per line per text: a markdown link writes the same reference
    // twice (its label and its target), and a reader sees one link. The text is
    // the reference AS WRITTEN, so two repositories citing the same number on one
    // line stay two findings.
    let mut seen: Vec<(usize, String)> = Vec::new();
    for (n, line) in prose_of(text) {
        for (reference, in_link) in scan_line(&line) {
            let (text, weight) = match reference.kind {
                RefKind::Register
                    if owns_room(root, &reference)
                        && resolution_of(root, &reference) == Resolution::UnresolvedHere =>
                {
                    (format!("{}/{}", reference.room, reference.number), Weight::DeadPointer)
                }
                // Written outside a link, so the site renders it as text whether
                // or not it names its repository. Calling a qualified reference
                // bare was false about what had been counted, and told an author
                // to rewrite text already in the recommended form.
                RefKind::Issue if !in_link => (
                    format!("{}#{}", reference.repo, reference.number),
                    Weight::Unrendered,
                ),
                _ => continue,
            };
            if seen.contains(&(n, text.clone())) {
                continue;
            }
            seen.push((n, text.clone()));
            out.push(Finding { file: file.to_string(), line: n, text, weight });
        }
    }
    out
}

/// `refs --check <dir>`: sweep the authored markdown, report what a reader cannot
/// follow, and settle the verdict. `0` clean, `3` advisory, `1` on any dead
/// pointer, `2` on a usage error.
pub fn refs_check(root: &Path) -> i32 {
    if !root.is_dir() {
        eprintln!("host-lifecycle: {} is not a directory", root.display());
        return 2;
    }
    let corpus = match crate::authored_corpus(root) {
        Ok(c) => c,
        Err(why) => {
            eprintln!("host-lifecycle: {why}");
            return 2;
        }
    };
    let docs = corpus.docs;
    if docs.is_empty() {
        // A clean verdict over nothing is the fail-unsafe shape: a cold reader
        // cannot tell it from a real pass.
        eprintln!(
            "host-lifecycle: no authored markdown to sweep under {}{}",
            root.display(),
            if corpus.excluded > 0 {
                format!(" ({} document(s) excluded by .host-lintignore or the record layer)", corpus.excluded)
            } else {
                " (not a git repository, or nothing tracked?)".to_string()
            }
        );
        return 2;
    }
    let rooms_here: Vec<&str> = ROOMS.iter().copied().filter(|r| root.join(r).is_dir()).collect();
    let mut findings: Vec<Finding> = Vec::new();
    let mut unchecked_registers = 0usize;
    let mut unread: Vec<String> = Vec::new();
    for doc in &docs {
        // A listed document that will not open is a hole in the corpus, never a
        // silent skip: the run cannot say what was in it, and a dead pointer
        // inside one shipped as a clean verdict at exit zero.
        let Ok(text) = fs::read_to_string(root.join(doc)) else {
            unread.push(doc.clone());
            continue;
        };
        unchecked_registers += count_unowned_registers(&text, root);
        findings.extend(scan_document(&text, doc, root));
    }
    for doc in &unread {
        println!("UNREAD   {doc}: listed by the walk and could not be read");
    }
    let dead: Vec<&Finding> = findings.iter().filter(|f| f.weight == Weight::DeadPointer).collect();
    for f in &dead {
        println!("DEAD     {}:{} {} names no entry in that room", f.file, f.line, f.text);
    }
    let debt: Vec<&Finding> = findings.iter().filter(|f| f.weight == Weight::Unrendered).collect();
    // Counted per file rather than listed line by line: a wall of bare numbers
    // printed in full is a wall nobody reads.
    let mut by_file: Vec<(String, usize)> = Vec::new();
    for f in &debt {
        match by_file.iter_mut().find(|(name, _)| *name == f.file) {
            Some((_, n)) => *n += 1,
            None => by_file.push((f.file.clone(), 1)),
        }
    }
    by_file.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (file, n) in by_file.iter().take(10) {
        println!("bare     {file}: {n} issue number(s) written outside a link");
    }
    if by_file.len() > 10 {
        println!("bare     … and {} more file(s)", by_file.len() - 10);
    }
    let swept = docs.len() - unread.len();
    if !dead.is_empty() || !unread.is_empty() {
        println!("-- {swept} doc(s) read of {} listed.", docs.len());
        if let Some(first) = dead.first() {
            println!(
                "   {} dead pointer(s): a reference naming a record that does not exist. Run `host-lifecycle resolve {} {}` to see where one points.",
                dead.len(),
                first.text,
                root.display()
            );
        }
        if !unread.is_empty() {
            println!(
                "   {} document(s) could not be read, so nothing here vouches for what is in them. Fix the permissions or the encoding and sweep again.",
                unread.len()
            );
        }
        disclose_uncovered(corpus.excluded, unchecked_registers, &rooms_here);
        return 1;
    }
    if !debt.is_empty() {
        println!(
            "-- {} doc(s) swept. {} issue reference(s) in {} file(s) written outside a link, so the site renders them as text rather than as links.",
            swept,
            debt.len(),
            by_file.len()
        );
        let unqualified = debt.iter().filter(|f| f.text.starts_with('#')).count();
        if unqualified > 0 {
            println!("   {unqualified} of them name no repository, so a reader cannot tell whose tracker they mean.");
        }
        println!("   Advisory: nothing is blocked. No flag fixes this; each reference is an edit.");
        // The worked example is a reference this tree already carries in qualified
        // form. It is never manufactured from a bare `#N`: prepending the origin
        // guesses the tracker, because in a host repository most bare numbers name a
        // component's issues while the origin remote is the host, and the weak-agent
        // acceptance is the evidence that a printed command gets run verbatim. The
        // placeholder spelling is no better; that was pasted verbatim too.
        println!("   Write each as owner/repo#N inside a link.");
        // Only an `owner/repo#N` example is offered. A bare component name would need a
        // recipe lookup to be resolvable, and the scanner also reads a written range
        // like `#10-#12` as the repository `10-`, so an example chosen on "not bare"
        // alone can print a command that does not resolve. A command this tool prints
        // is a command that gets run.
        match debt.iter().find(|f| f.text.contains('/')) {
            Some(example) => println!(
                "   For one of them: host-lifecycle resolve {} --markdown {}",
                example.text,
                root.display()
            ),
            None => println!(
                "   Every one of them names no repository, so only you know which tracker each meant."
            ),
        }
        disclose_uncovered(corpus.excluded, unchecked_registers, &rooms_here);
        return 3;
    }
    println!("-- {swept} doc(s) swept; every reference in them resolves and renders");
    disclose_uncovered(corpus.excluded, unchecked_registers, &rooms_here);
    0
}

/// What the sweep did not check, printed on every verdict it can reach. Printing
/// it on the clean branch alone meant it never printed in a software repository,
/// which carries legibility debt almost always and is the case it exists for.
fn disclose_uncovered(excluded: usize, unchecked_registers: usize, rooms_here: &[&str]) {
    if excluded > 0 {
        println!(
            "   {excluded} document(s) were excluded and not swept (the record layer, and whatever .host-lintignore names); they are records, so nothing here asks for them to be rewritten"
        );
    }
    // The room names are the ones this methodology fixes. A project that renamed
    // its rooms, or nests them under a subdirectory, had every register reference
    // pass through unseen, and the verdict then read as a clean bill over a
    // corpus the grammar never recognised.
    if rooms_here.is_empty() {
        println!(
            "   no {} room here, so no register reference was checked against anything; this is ordinary in a software repository and a defect in a host whose rooms are named or nested differently",
            ROOMS.map(|r| format!("{r}/")).join(" or ")
        );
    }
    if unchecked_registers > 0 {
        println!(
            "   {unchecked_registers} register reference(s) name a room this repository does not hold (rooms here: {}); they belong to its governing host and were not checked",
            if rooms_here.is_empty() { "none".to_string() } else { rooms_here.join(", ") }
        );
    }
}

/// An issue reference this tree already carries in qualified form, for use as a worked
/// example. `None` when every issue reference here is bare, in which case no command is
/// demonstrated rather than one being manufactured.
fn first_qualified_reference(root: &Path) -> Option<String> {
    for doc in crate::authored_docs(root) {
        let Ok(text) = fs::read_to_string(root.join(&doc)) else { continue };
        for (_, line) in prose_of(&text) {
            for (reference, _) in scan_line(&line) {
                // `owner/repo#N` only, for the reason the advisory summary gives: a bare
                // component name needs a lookup to resolve, and a written range reads as
                // a repository, so either could print a command that does not work.
                if reference.kind == RefKind::Issue && reference.repo.contains('/') {
                    return Some(format!("{}#{}", reference.repo, reference.number));
                }
            }
        }
    }
    None
}

/// References this repository cannot check, because it does not own their room.
/// Counted rather than reported: they are somebody else's registers, and a clean
/// line that did not mention them would claim coverage it does not have.
fn count_unowned_registers(text: &str, root: &Path) -> usize {
    let mut n = 0;
    for (_, line) in prose_of(text) {
        for (reference, _) in scan_line(&line) {
            if reference.kind == RefKind::Register && !owns_room(root, &reference) {
                n += 1;
            }
        }
    }
    n
}

/// `host-lifecycle resolve <ref> [--markdown|--url] [<dir>]`.
pub fn resolve(args: &[String]) {
    let mut emission = Emission::Path;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--markdown" => emission = Emission::MarkdownLink,
            "--url" => emission = Emission::FullUrl,
            "--path" => emission = Emission::Path,
            // An unrecognised flag became the directory, so a mistyped `--md` was
            // reported as a reference that does not resolve — a false verdict
            // wearing the governing-host explanation.
            _ if a.starts_with("--") => {
                eprintln!("host-lifecycle: unknown flag `{a}` (expected --markdown, --url or --path)");
                process::exit(2);
            }
            _ => pos.push(a),
        }
    }
    let Some(reference_text) = pos.first() else {
        eprintln!("host-lifecycle resolve <plan/NNNN|call/NNNN|#N>[#anchor] [--markdown|--url] [<dir>]");
        process::exit(2);
    };
    let root = match pos.get(1) {
        Some(d) => PathBuf::from(d.as_str()),
        None => PathBuf::from("."),
    };
    // A root that cannot be read is a usage error. Reporting it as a resolution
    // outcome told the caller the reference was unresolvable when the directory
    // was simply not there.
    if !root.is_dir() {
        eprintln!("host-lifecycle: {} is not a directory", root.display());
        process::exit(2);
    }
    if resolution(&root, reference_text) == Resolution::Malformed {
        eprintln!("host-lifecycle: `{reference_text}` is not a reference (expected plan/NNNN, call/NNNN or owner/repo#N)");
        // Named from this tree rather than left as a shape to fill in. The
        // weak-agent probe truncated a long remedy to `resolve --markdown .`,
        // dropping the reference; a usage error alone teaches nothing, and the
        // `--fix` refusal already proved that naming a real one gets it run.
        if let Some(real) = first_register_reference(&root) {
            eprintln!("  A reference from this tree: host-lifecycle resolve {real} --markdown {}", root.display());
        }
        process::exit(2);
    }
    let reference = parse_reference(reference_text).expect("malformed was answered above");
    match emit(&root, &reference, emission) {
        Ok(text) => println!("{text}"),
        Err(why) => {
            eprintln!("host-lifecycle: {why}");
            process::exit(1);
        }
    }
}

/// A register reference this tree actually holds, so a usage error can name one
/// rather than describe its shape.
fn first_register_reference(root: &Path) -> Option<String> {
    for room in ROOMS {
        let mut entries: Vec<String> = fs::read_dir(root.join(room))
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .filter(|n| n.len() > 4 && n[..4].chars().all(|c| c.is_ascii_digit()) && n[4..].starts_with('-'))
            .collect();
        entries.sort();
        if let Some(first) = entries.first() {
            return Some(format!("{room}/{}", &first[..4]));
        }
    }
    None
}

/// The first bare issue reference in the tree, as (file, `#N`), so a refusal can
/// name a real one instead of a placeholder.
fn first_bare_reference(root: &Path) -> Option<(String, String)> {
    for doc in crate::authored_docs(root) {
        let Ok(text) = fs::read_to_string(root.join(&doc)) else { continue };
        if let Some(f) = scan_document(&text, &doc, root).into_iter().find(|f| f.weight == Weight::Unrendered) {
            return Some((f.file, f.text));
        }
    }
    None
}

/// `host-lifecycle refs --check <dir>`.
pub fn refs(args: &[String]) {
    let mut check = false;
    let mut fix = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "--fix" => fix = true,
            _ if a.starts_with("--") => {
                eprintln!("host-lifecycle: unknown flag `{a}` (expected --check)");
                process::exit(2);
            }
            _ => pos.push(a),
        }
    }
    // `--fix` exists only to refuse, with the reason. The weak-agent probe read a
    // report of 293 bare references and typed `refs --fix` twice, against a line
    // that said no flag fixes this; a usage error would have taught it nothing, so
    // the flag answers the question it was really asking.
    if fix {
        let root = pos.first().map(|d| PathBuf::from(d.as_str())).unwrap_or_else(|| PathBuf::from("."));
        eprintln!(
            "host-lifecycle: there is no --fix for references. A bare `#N` names no repository, and only the author knows which tracker it meant: rewriting it would be guessing."
        );
        // Named from this tree, never written as a placeholder: the weak-agent probe
        // pasted `owner/repo#N` verbatim when the refusal spelled it that way. The
        // location comes from a real bare reference, and the runnable command is
        // demonstrated on a reference the tree already carries qualified, because
        // qualifying the bare one would be the guess this refusal exists to refuse.
        match first_bare_reference(&root) {
            Some((file, text)) => {
                eprintln!("  Start with `{text}` in {file}; only you can say whose tracker it names.");
                if let Some(example) = first_qualified_reference(&root) {
                    eprintln!(
                        "  Written as owner/repo#N it resolves, as this one already does: host-lifecycle resolve {} --markdown {}",
                        example,
                        root.display()
                    );
                }
            }
            None => eprintln!("  This tree has no bare issue reference to rewrite."),
        }
        process::exit(2);
    }
    if !check {
        eprintln!("host-lifecycle refs --check <dir>");
        process::exit(2);
    }
    let root = pos.first().map(|d| PathBuf::from(d.as_str())).unwrap_or_else(|| PathBuf::from("."));
    process::exit(refs_check(&root));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("hl-refs-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("plan").join("0074-materialize")).unwrap();
        fs::write(base.join("plan").join("0074-materialize").join("README.md"), "# m\n").unwrap();
        fs::create_dir_all(base.join("call")).unwrap();
        fs::write(base.join("call").join("0045-store-model.md"), "# d\n").unwrap();
        base
    }

    // A reference is a room, four digits, and an optional anchor. A near-miss is
    // not a reference at all, which keeps a typo from resolving somewhere else.
    #[test]
    fn parses_the_reference_shapes() {
        assert_eq!(parse_reference("plan/0074").unwrap().number, "0074");
        assert_eq!(parse_reference("call/0045").unwrap().room, "call");
        assert_eq!(parse_reference("plan/0074#write-spec").unwrap().anchor.unwrap(), "write-spec");
        assert_eq!(parse_reference("#17").unwrap().kind, RefKind::Issue);
        assert!(parse_reference("plan/074").is_none(), "three digits is not the register's shape");
        assert!(parse_reference("plan/00741").is_none(), "nor is five");
        assert!(parse_reference("notaroom/0074").is_none());
        assert!(parse_reference("#abc").is_none());
    }

    // A milestone resolves to its README, a decision to its file, and the anchor
    // rides along so a task node lands on its heading.
    #[test]
    fn resolves_a_register_reference_to_its_entry() {
        let base = fixture("resolve");
        let milestone = parse_reference("plan/0074#write-spec").unwrap();
        let path = emit(&base, &milestone, Emission::Path).unwrap();
        assert_eq!(path, "plan/0074-materialize/README.md#write-spec");
        let link = emit(&base, &milestone, Emission::MarkdownLink).unwrap();
        assert_eq!(link, "[plan/0074#write-spec](plan/0074-materialize/README.md#write-spec)");
        let decision = parse_reference("call/0045").unwrap();
        assert_eq!(emit(&base, &decision, Emission::Path).unwrap(), "call/0045-store-model.md");
        // The path the resolution rests on, and the absence that is not one.
        assert!(entry_path(&base, &decision).is_some());
        assert!(entry_path(&base, &parse_reference("call/0099").unwrap()).is_none());
        let _ = fs::remove_dir_all(&base);
    }

    // A number the room does not hold is unresolved HERE, and the message says
    // which of the three situations it is. One sentence for all three diagnosed a
    // repository that OWNS the room as one that does not, so a wrong working
    // directory read as a wrong repository.
    #[test]
    fn names_which_unresolved_case_this_is() {
        let base = fixture("unresolved");
        // The room is here and holds no such number.
        let absent = parse_reference("plan/0099").unwrap();
        let err = emit(&base, &absent, Emission::Path).unwrap_err();
        assert!(err.contains("unresolved here"), "{err}");
        assert!(err.contains("plan/ exists"), "the room is here, so say so: {err}");
        assert!(!err.contains("governing host"), "and do not blame a governing host: {err}");

        // No room at all: this IS the governing-host case, and the only one.
        let bare = base.join("software");
        fs::create_dir_all(&bare).unwrap();
        let err = emit(&bare, &absent, Emission::Path).unwrap_err();
        assert!(err.contains("governing host"), "{err}");
        assert!(err.contains("no plan/ room"), "{err}");

        // Two entries share a number: an ambiguity, never a silent first match.
        fs::create_dir_all(base.join("plan").join("0074-abandoned-draft")).unwrap();
        fs::write(base.join("plan").join("0074-abandoned-draft").join("README.md"), "# d\n").unwrap();
        let doubled = parse_reference("plan/0074").unwrap();
        let err = emit(&base, &doubled, Emission::Path).unwrap_err();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("0074-abandoned-draft") && err.contains("0074-materialize"), "names both: {err}");
        let _ = fs::remove_dir_all(&base);
    }

    // A milestone whose record is the directory resolves to the directory. The
    // README existence gate reported a milestone that is plainly there as a dead
    // pointer, and it gated.
    #[test]
    fn a_milestone_without_a_readme_is_still_the_entry() {
        let base = fixture("noreadme");
        fs::create_dir_all(base.join("plan").join("0080-directory-is-the-record")).unwrap();
        fs::write(base.join("plan").join("0080-directory-is-the-record").join("design.md"), "# d\n").unwrap();
        let reference = parse_reference("plan/0080").unwrap();
        assert_eq!(
            emit(&base, &reference, Emission::Path).unwrap(),
            "plan/0080-directory-is-the-record"
        );
        let found = scan_document("governed by plan/0080\n", "README.md", &base);
        assert!(found.is_empty(), "the entry is there, so it is not a dead pointer: {found:?}");
        let _ = fs::remove_dir_all(&base);
    }

    // A dead pointer gates; a bare issue number advises; a reference inside a link
    // or inside fenced code is neither.
    #[test]
    fn sweeps_dead_pointers_and_bare_issue_numbers() {
        let base = fixture("sweep");
        let doc = "see plan/0074 and call/0045\n\
                   a dead one: plan/0099\n\
                   bare #17 here\n\
                   linked [#18](https://github.com/o/r/issues/18) there\n\
                   ```\n\
                   fenced #19 and plan/0098\n\
                   ```\n";
        let found = scan_document(doc, "doc.md", &base);
        let dead: Vec<&Finding> = found.iter().filter(|f| f.weight == Weight::DeadPointer).collect();
        let debt: Vec<&Finding> = found.iter().filter(|f| f.weight == Weight::Unrendered).collect();
        assert_eq!(dead.len(), 1, "one dead pointer: {found:?}");
        assert_eq!(dead[0].text, "plan/0099");
        assert_eq!(debt.len(), 1, "one bare issue number: {found:?}");
        assert_eq!(debt[0].text, "#17");
        // The linked issue on line 4 is not debt: `enclosing_link` reports it as
        // in_link, and a reference that renders is not reported.
        assert!(found.iter().all(|f| f.text != "#18"), "a linked issue renders: {found:?}");
        let linked: Vec<(Reference, bool)> = scan_line("[#18](https://github.com/o/r/issues/18)");
        assert!(linked.iter().any(|(r, in_link)| r.number == "18" && *in_link));
        assert!(found.iter().all(|f| f.line != 6), "fenced references are examples, never findings");

        // Quoted in backticks is shown rather than referred, for either kind; a
        // register reference inside a LINK is still checked, because a dead
        // pointer wrapped in a link is still dead.
        let quoted = scan_document("an example: `plan/0098` and `#21`\n", "doc.md", &base);
        assert!(quoted.is_empty(), "inline code quotes rather than refers: {quoted:?}");
        let linked_dead = scan_document("[plan/0097](plan/0097-x/README.md)\n", "doc.md", &base);
        assert_eq!(linked_dead.len(), 1, "a dead pointer inside a link is still dead");
        let _ = fs::remove_dir_all(&base);
    }

    // A repository that does not own the room is not the reference's host: its
    // documents cite the numbers that govern it, and a sweep that called those
    // dead would redden every software repository for doing so.
    #[test]
    fn a_repository_without_the_room_owns_no_dead_pointer() {
        let base = std::env::temp_dir().join(format!("hl-refs-noroom-{}", process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let reference = parse_reference("call/0045").unwrap();
        assert!(!owns_room(&base, &reference), "no room here, so this repository owns no such pointer");
        let found = scan_document("governed by call/0045 and plan/0074\n", "README.md", &base);
        assert!(found.is_empty(), "so nothing is reported: {found:?}");
        let _ = fs::remove_dir_all(&base);
    }

    // The three outcomes, asked of the text as written: a reference this room
    // holds, one it does not, and text that is not a reference at all.
    #[test]
    fn reports_the_three_resolution_outcomes() {
        let base = fixture("outcomes");
        assert_eq!(resolution(&base, "plan/0074"), Resolution::Resolved);
        assert_eq!(resolution_of(&base, &parse_reference("plan/0074").unwrap()), Resolution::Resolved);
        assert_eq!(resolution_of(&base, &parse_reference("plan/0099").unwrap()), Resolution::UnresolvedHere);
        assert_eq!(resolution(&base, "plan/0099"), Resolution::UnresolvedHere);
        assert_eq!(resolution(&base, "plan/74"), Resolution::Malformed);
        assert_eq!(resolution(&base, "not a reference"), Resolution::Malformed);
        let _ = fs::remove_dir_all(&base);
    }

    // A document is read the way a markdown reader reads it. Each of these was a
    // false verdict: the first three gated correct documents, and the last two
    // lost a dead pointer the gate exists to catch.
    #[test]
    fn quoting_is_read_the_way_a_markdown_reader_reads_it() {
        let base = fixture("blocks");
        let cases: [(&str, usize, &str); 6] = [
            ("> ```\n> an example: #19 and plan/0097\n> ```\n", 0, "a fence inside a blockquote is still a fence"),
            ("prose\n\n    host-lifecycle resolve plan/0097 .\n", 0, "an indented code block is code"),
            ("<!-- TODO: renumber plan/0097 -->\n", 0, "an HTML comment is not prose"),
            ("The gate would force a `cargo fmt\n--check` run, so plan/0097 stayed dead.\n", 1, "a code span wrapping a line break quotes only itself"),
            ("Don't write ` there, and note plan/0097 is dead.\n", 1, "an unmatched backtick is literal and quotes nothing"),
            ("``an example: #19 inside a double span``\n", 0, "a nested span quotes what it holds"),
        ];
        for (doc, expected, why) in cases {
            let found = scan_document(doc, "doc.md", &base);
            assert_eq!(found.len(), expected, "{why}: {found:?}");
        }
        // An indented continuation under a bullet is prose, not a code block, so
        // reading it as code would have lost coverage rather than gained it.
        let listed = scan_document("- a bullet\n    continued, and plan/0097 is dead\n", "doc.md", &base);
        assert_eq!(listed.len(), 1, "a list continuation is prose: {listed:?}");
        let _ = fs::remove_dir_all(&base);
    }

    // The sweep counted a fully-qualified reference as bare and said it named no
    // repository, which was false about what it had counted and told an author to
    // rewrite text already in the recommended form.
    #[test]
    fn a_qualified_reference_is_not_called_bare() {
        let base = fixture("qualified");
        let found = scan_document("compare host-lint#17 with host-prove#17 on one line\n", "doc.md", &base);
        assert_eq!(found.len(), 2, "two repositories are two references: {found:?}");
        assert_eq!(found[0].text, "host-lint#17");
        assert_eq!(found[1].text, "host-prove#17");
        // A URL fragment is not an issue number at all.
        let fragment = scan_document("see https://example.com/spec/page#2024 for the shape\n", "doc.md", &base);
        assert!(fragment.is_empty(), "a URL fragment names no issue: {fragment:?}");
        // A pair of review-finding codes is not a repository and a number: the
        // walk-back crossed the slash and read `41/` as one, so `#50` was
        // counted as an issue in somebody's tracker.
        let codes = scan_document("the deferred #41/#50 dogfood\n", "doc.md", &base);
        assert_eq!(codes.len(), 1, "one bare number, not a repository reference: {codes:?}");
        assert_eq!(codes[0].text, "#41");
        let _ = fs::remove_dir_all(&base);
    }

    // A component's owner comes from what the tree RECORDS, never from a guess at
    // the local origin: `.gitmodules` here names another owner, and guessing sent
    // the reader to a repository that does not exist.
    #[test]
    fn a_component_takes_the_owner_this_tree_records() {
        let base = fixture("owner");
        fs::write(base.join(".gitmodules"), "[submodule \"tools/allium\"]\n\turl = https://github.com/juxt/allium.git\n").unwrap();
        assert_eq!(recorded_owner(&base, "allium").as_deref(), Some("juxt"));
        assert_eq!(recorded_owner(&base, "not-a-component"), None);
        let _ = fs::remove_dir_all(&base);
    }

    // The forge slugifies a whole heading line, braces and all, so the explicit
    // id is translated rather than pasted: emitting `#write-spec` produced a
    // fragment the forge does not have and dropped the reader at the top.
    #[test]
    fn the_url_anchor_is_the_one_the_forge_actually_holds() {
        let base = fixture("anchor");
        let entry = base.join("plan").join("0074-materialize").join("README.md");
        fs::write(&entry, "# m\n\n### The reference surface {#write-spec}\n").unwrap();
        assert_eq!(
            github_anchor(&entry, "write-spec").as_deref(),
            Some("the-reference-surface-write-spec")
        );
        // A heading the document does not carry yields no fragment, rather than a
        // confident one that matches nothing.
        assert_eq!(github_anchor(&entry, "no-such-node"), None);
        let _ = fs::remove_dir_all(&base);
    }

    // The two facts a resolved reference must not lose: its anchor, and the
    // honesty of a URL it cannot build.
    #[test]
    fn url_needs_an_origin_and_the_anchor_survives() {
        let base = fixture("url");
        let anchored = parse_reference("plan/0074#write-spec").unwrap();
        for emission in [Emission::Path, Emission::MarkdownLink] {
            assert!(emit(&base, &anchored, emission).unwrap().contains("#write-spec"));
        }
        // The fixture is not a git repository, so no origin can be read, and a URL
        // is the one emission that needs one.
        assert!(origin_slug(&base).is_none());
        let err = emit(&base, &anchored, Emission::FullUrl).unwrap_err();
        assert!(err.contains("origin"), "{err}");
        let issue = parse_reference("#17").unwrap();
        assert!(emit(&base, &issue, Emission::Path).unwrap_err().contains("forge"));
        let _ = fs::remove_dir_all(&base);
    }
}
