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
    /// `plan` or `call` for a register reference; the issue number's repository
    /// is not carried by the reference, so this is empty for an issue.
    pub room: String,
    pub number: String,
    pub anchor: Option<String>,
}

/// Parse a reference as written. `plan/0074`, `call/0045`, `plan/0074#write-spec`
/// and `#17` are references; anything else is not.
pub fn parse_reference(text: &str) -> Option<Reference> {
    let t = text.trim();
    if let Some(n) = t.strip_prefix('#') {
        if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return Some(Reference {
            kind: RefKind::Issue,
            room: String::new(),
            number: n.to_string(),
            anchor: None,
        });
    }
    let (room, rest) = t.split_once('/')?;
    if !ROOMS.contains(&room) {
        return None;
    }
    let (number, anchor) = match rest.split_once('#') {
        Some((n, a)) => (n, Some(a.to_string())),
        None => (rest, None),
    };
    // Four digits is the register's shape; `plan/074` and `plan/00741` are not
    // references, so a typo reads as malformed rather than resolving elsewhere.
    if number.len() != 4 || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(Reference { kind: RefKind::Register, room: room.to_string(), number: number.to_string(), anchor })
}

/// The path a register reference names: a milestone directory's README, or a
/// decision file. `None` when the room holds no entry with that number.
pub fn entry_path(root: &Path, reference: &Reference) -> Option<PathBuf> {
    if reference.kind != RefKind::Register {
        return None;
    }
    let room = root.join(&reference.room);
    let mut matches: Vec<PathBuf> = fs::read_dir(&room)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.strip_prefix(&reference.number).is_some_and(|r| r.starts_with('-')))
        })
        .collect();
    matches.sort();
    let hit = matches.into_iter().next()?;
    // A milestone is a directory whose README is the page; a decision is a file.
    let path = if hit.is_dir() { hit.join("README.md") } else { hit };
    path.exists().then_some(path)
}

/// The repository's forge coordinates, from the origin remote: `owner/repo`.
/// `None` in a repository with no remote, which is why a URL can fail to build
/// while a path cannot.
pub fn origin_slug(root: &Path) -> Option<String> {
    let url = crate::git_out(root, &["remote", "get-url", "origin"])?;
    let url = url.trim().trim_end_matches(".git");
    let rest = url
        .rsplit_once("github.com")
        .map(|(_, r)| r.trim_start_matches([':', '/']))?;
    (rest.matches('/').count() == 1).then(|| rest.to_string())
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
        RefKind::Issue => match origin_slug(root) {
            Some(_) => Resolution::Resolved,
            None => Resolution::UnresolvedHere,
        },
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
            let slug = origin_slug(root)
                .ok_or_else(|| "no github origin remote, so an issue number cannot become a URL".to_string())?;
            match emission {
                Emission::FullUrl => Ok(format!("https://github.com/{slug}/issues/{}", reference.number)),
                Emission::MarkdownLink => Ok(format!(
                    "[{}#{}](https://github.com/{slug}/issues/{})",
                    slug, reference.number, reference.number
                )),
                Emission::Path => unreachable!("answered above, before the remote is read"),
            }
        }
        RefKind::Register => {
            let rel = entry_path(root, reference).ok_or_else(|| {
                format!(
                    "unresolved here: {}/{} names no entry in {}/ (in a software repository the number belongs to its governing host)",
                    reference.room, reference.number, reference.room
                )
            })?;
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
                    Ok(format!("https://github.com/{slug}/blob/main/{rel}{anchor}"))
                }
            }
        }
    }
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
fn scan_line(line: &str) -> Vec<(Reference, bool, bool)> {
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
                            number: digits,
                            anchor,
                        },
                        enclosing_link(&bytes, i, end),
                        in_inline_code(&bytes, i),
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
        if bytes[i] == '#'
            && (i == 0 || !(bytes[i - 1].is_alphanumeric() || bytes[i - 1] == '/' || bytes[i - 1] == '-'))
        {
            let digits: String = bytes[i + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                let end = i + 1 + digits.chars().count();
                out.push((
                    Reference { kind: RefKind::Issue, room: String::new(), number: digits, anchor: None },
                    enclosing_link(&bytes, i, end),
                    in_inline_code(&bytes, i),
                ));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Whether the span sits inside inline code. Backticks quote: a reference written
/// there is being SHOWN, not made, which is why a document that teaches the rule
/// does not fail it. The same reading the tell gate gives its own fixtures.
fn in_inline_code(line: &[char], start: usize) -> bool {
    line[..start].iter().filter(|c| **c == '`').count() % 2 == 1
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
    let mut fenced = false;
    // One reference per line per text: a markdown link writes the same reference
    // twice (its label and its target), and a reader sees one link.
    let mut seen: Vec<(usize, String)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        for (reference, in_link, in_code) in scan_line(line) {
            // Quoted is shown, not referred: a reference in inline code is an
            // example, whichever kind it is.
            if in_code {
                continue;
            }
            let (text, weight) = match reference.kind {
                RefKind::Register
                    if owns_room(root, &reference)
                        && resolution_of(root, &reference) == Resolution::UnresolvedHere =>
                {
                    (format!("{}/{}", reference.room, reference.number), Weight::DeadPointer)
                }
                RefKind::Issue if !in_link => (format!("#{}", reference.number), Weight::Unrendered),
                _ => continue,
            };
            if seen.contains(&(n, text.clone())) {
                continue;
            }
            seen.push((n, text.clone()));
            out.push(Finding { file: file.to_string(), line: n + 1, text, weight });
        }
    }
    out
}

/// `refs --check <dir>`: sweep the authored markdown, report what a reader cannot
/// follow, and settle the verdict. `0` clean, `3` advisory, `1` on any dead
/// pointer, `2` on a usage error.
pub fn refs_check(root: &Path) -> i32 {
    let docs = crate::authored_docs(root);
    let mut findings: Vec<Finding> = Vec::new();
    for doc in &docs {
        let Ok(text) = fs::read_to_string(root.join(doc)) else { continue };
        findings.extend(scan_document(&text, doc, root));
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
    if !dead.is_empty() {
        println!(
            "-- {} dead pointer(s): a reference naming a record that does not exist. Fix the number or the reference; run `host-lifecycle resolve <ref> {}` to see where one points.",
            dead.len(),
            root.display()
        );
        return 1;
    }
    if !debt.is_empty() {
        println!(
            "-- {} bare issue reference(s) in {} file(s). Advisory: the site cannot render them and a reader outside the forge cannot follow them. Write each as a markdown link: `host-lifecycle resolve '#N' --markdown {}`.",
            debt.len(),
            by_file.len(),
            root.display()
        );
        return 3;
    }
    println!("-- every reference in the authored docs resolves and renders");
    0
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
    if resolution(&root, reference_text) == Resolution::Malformed {
        eprintln!("host-lifecycle: `{reference_text}` is not a reference (expected plan/NNNN, call/NNNN or #N)");
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

/// `host-lifecycle refs --check <dir>`.
pub fn refs(args: &[String]) {
    let mut check = false;
    let mut pos: Vec<&String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            _ => pos.push(a),
        }
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

    // A number the room does not hold is unresolved HERE: the message says which
    // room was searched and why a software repository sees this normally, rather
    // than guessing at another repository's registers.
    #[test]
    fn reports_unresolved_here_rather_than_guessing() {
        let base = fixture("unresolved");
        let absent = parse_reference("plan/0099").unwrap();
        let err = emit(&base, &absent, Emission::Path).unwrap_err();
        assert!(err.contains("unresolved here"), "{err}");
        assert!(err.contains("governing host"), "and says why a software repo sees it: {err}");
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
        let linked: Vec<(Reference, bool, bool)> = scan_line("[#18](https://github.com/o/r/issues/18)");
        assert!(linked.iter().any(|(r, in_link, _)| r.number == "18" && *in_link));
        assert!(found.iter().all(|f| f.line != 6), "fenced references are examples, never findings");

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

    // Quoted in backticks is shown rather than referred, for either kind; a
        // register reference inside a LINK is still checked, because a dead
        // pointer wrapped in a link is still dead.
        let quoted = scan_document("an example: `plan/0098` and `#21`\n", "doc.md", &base);
        assert!(quoted.is_empty(), "inline code quotes rather than refers: {quoted:?}");
        let linked_dead = scan_document("[plan/0097](plan/0097-x/README.md)\n", "doc.md", &base);
        assert_eq!(linked_dead.len(), 1, "a dead pointer inside a link is still dead");
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
