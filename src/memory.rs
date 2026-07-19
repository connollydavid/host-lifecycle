//! The host-* per-user memory store (plan/0073).
//!
//! Markdown-per-entry storage at `~/.host-memory/<project>/`, with a
//! `MEMORY.md` index and `[[slug]]` cross-entry links. The repo `MEMORY.md`
//! is the append-only tier (governed by CLAUDE.md section 6, read elsewhere);
//! this module owns the editable per-user tier that `host-lifecycle dream`
//! audits and that the MCP `memory_*` tools (plan/0065 + plan/0073) read and
//! write.
//!
//! Format (the methodology's own; spine doctrine lands at plan/0073
//! #write-spine-doctrine):
//!
//! ```text
//! ~/.host-memory/<encoded-cwd>/
//!   MEMORY.md                  # the index: one bullet per entry
//!   <slug>.md                  # one markdown file per memory entry
//!   ...
//! ```
//!
//! Each entry file is YAML frontmatter + free-form markdown body:
//!
//! ```text
//! ---
//! description: <one-line summary; what recall keys on>
//! type: feedback | fact | workaround | state
//! created: YYYY-MM-DD
//! last_edited: YYYY-MM-DD
//! superseded_by: <slug or empty>
//! ---
//!
//! <body>
//! ```
//!
//! The index is one bullet per entry: `- [<Title>](<slug>.md): <description>`.
//! Title and slug may coincide; the description mirrors the frontmatter line
//! so recall and index agree.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The entry's class, mirroring the prose taxonomy the audit and the cast
/// review settle on. `Feedback` is the operator-preference / correction class;
/// `Fact` is durable measured lore; `Workaround` is a fix-of-the-moment (the
/// detector that distinguishes it from `Fact` is `workaround-vs-plan`);
/// `State` is a snapshot that may go stale (the detector `stale-state-over
/// -lore` keeps the durable bits when the state flips).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryType {
    Feedback,
    Fact,
    Workaround,
    State,
}

impl EntryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryType::Feedback => "feedback",
            EntryType::Fact => "fact",
            EntryType::Workaround => "workaround",
            EntryType::State => "state",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim() {
            "feedback" => Ok(EntryType::Feedback),
            "fact" => Ok(EntryType::Fact),
            "workaround" => Ok(EntryType::Workaround),
            "state" => Ok(EntryType::State),
            other => Err(format!("unknown entry type: {other}")),
        }
    }
}

/// One memory entry, in either store. The structural fields mirror the
/// Allium spec; the audit populates the detector-precondition booleans from
/// the entry's content and the rest of the store, and the dream rule set
/// fires when they hold. The store layer itself does not compute them.
///
/// Dates are `YYYY-MM-DD` strings (parsed/validated on read; no chrono dep,
/// matching the rest of host-lifecycle's date-via-Hinnant discipline).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryEntry {
    pub slug: String,
    pub description: String,
    pub body: String,
    pub entry_type: EntryType,
    pub created: String,
    pub last_edited: String,
    pub superseded_by: String,
}

impl MemoryEntry {
    /// Render the entry as the on-disk markdown file (YAML frontmatter + body).
    pub fn render(&self) -> String {
        let mut s = String::new();
        s.push_str("---\n");
        s.push_str(&format!("description: {}\n", yaml_escape(&self.description)));
        s.push_str(&format!("type: {}\n", self.entry_type.as_str()));
        s.push_str(&format!("created: {}\n", self.created));
        s.push_str(&format!("last_edited: {}\n", self.last_edited));
        s.push_str(&format!(
            "superseded_by: {}\n",
            yaml_escape(&self.superseded_by)
        ));
        s.push_str("---\n\n");
        s.push_str(&self.body);
        if !self.body.ends_with('\n') {
            s.push('\n');
        }
        s
    }

    /// Parse an entry from its on-disk markdown file content.
    pub fn parse(slug: &str, content: &str) -> Result<Self, String> {
        let (frontmatter, body) = split_frontmatter(content)
            .ok_or_else(|| format!("entry {slug}: missing YAML frontmatter"))?;
        let map = parse_frontmatter(frontmatter);
        let description = map
            .get("description")
            .cloned()
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        let entry_type = EntryType::parse(
            map.get("type").map(|s| s.as_str()).unwrap_or("fact"),
        )?;
        let created = parse_date(map.get("created").map(|s| s.as_str()).unwrap_or(""))?;
        let last_edited = parse_date(map.get("last_edited").map(|s| s.as_str()).unwrap_or(""))?;
        let superseded_by = map
            .get("superseded_by")
            .cloned()
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        Ok(MemoryEntry {
            slug: slug.to_string(),
            description,
            body: body.trim_start_matches('\n').to_string(),
            entry_type,
            created,
            last_edited,
            superseded_by,
        })
    }
}

/// A handle to a per-user memory store at `<root>/<encoded-cwd>/`. `root` is
/// `~/.host-memory` by default; tests pass a tmp dir. The handle is cheap to
/// clone; all state is on disk.
#[derive(Clone, Debug)]
pub struct MemoryStore {
    dir: PathBuf,
}

impl MemoryStore {
    /// Open (or conceptually create-on-first-write) the store for `project_cwd`
    /// under `root`. The directory and the index file are not materialized
    /// until the first write; a read over an absent store returns an empty
    /// entry list.
    pub fn open(root: &Path, project_cwd: &Path) -> Result<Self, String> {
        let encoded = encode_cwd(project_cwd);
        let dir = root.join(encoded);
        Ok(MemoryStore { dir })
    }

    /// The store's directory on disk (lazily materialized).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// List every entry, read from the index. Entries whose file is missing
    /// (the index/entry divergence failure mode) are skipped with a recorded
    /// dangling pointer; the dream audit catches them separately via
    /// `LinkRef.resolves = false`.
    pub fn list(&self) -> Result<Vec<MemoryEntry>, String> {
        let Some(index) = self.read_index()? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(index.len());
        for line in &index {
            if let Ok(entry) = self.read(&line.slug) {
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// Read one entry by slug. Errors if the file is missing or malformed.
    pub fn read(&self, slug: &str) -> Result<MemoryEntry, String> {
        let path = self.entry_path(slug);
        let content = fs::read_to_string(&path).map_err(|e| format!("read {slug}: {e}"))?;
        MemoryEntry::parse(slug, &content)
    }

    /// Write an entry: serialize to `<slug>.md`, then refresh the index so the
    /// new entry appears (or its description is corrected). Atomic per file:
    /// write to `<slug>.tmp` then rename, so a crash mid-write never leaves a
    /// truncated entry file.
    pub fn write(&self, entry: &MemoryEntry) -> Result<(), String> {
        self.ensure_dir()?;
        let path = self.entry_path(&entry.slug);
        let tmp = path.with_extension("md.tmp");
        let mut f = fs::File::create(&tmp).map_err(|e| format!("write tmp: {e}"))?;
        f.write_all(entry.render().as_bytes())
            .map_err(|e| format!("write tmp: {e}"))?;
        f.sync_all().map_err(|e| format!("sync tmp: {e}"))?;
        drop(f);
        fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
        self.upsert_index(entry)
    }

    /// Delete an entry: remove the file, then refresh the index. Idempotent:
    /// a missing file is a no-op.
    pub fn delete(&self, slug: &str) -> Result<(), String> {
        let path = self.entry_path(slug);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("delete {slug}: {e}"))?;
        }
        self.remove_from_index(slug)
    }

    /// Resolve a `[[slug]]` reference: true iff an entry file with that slug
    /// exists in this store. Same-store by format definition (plan/0073 spec,
    /// LinkRef entity); cross-store references are ordinary prose, not links.
    pub fn resolves(&self, target_slug: &str) -> bool {
        self.entry_path(target_slug).exists()
    }

    fn entry_path(&self, slug: &str) -> PathBuf {
        self.dir.join(format!("{slug}.md"))
    }

    fn ensure_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("mkdir {}: {e}", self.dir.display()))
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("MEMORY.md")
    }

    fn read_index(&self) -> Result<Option<Vec<IndexLine>>, String> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|e| format!("read index: {e}"))?;
        Ok(Some(parse_index(&content)))
    }

    fn write_index(&self, lines: &[IndexLine]) -> Result<(), String> {
        self.ensure_dir()?;
        let path = self.index_path();
        let tmp = path.with_extension("md.tmp");
        let mut f = fs::File::create(&tmp).map_err(|e| format!("write index tmp: {e}"))?;
        f.write_all(render_index(lines).as_bytes())
            .map_err(|e| format!("write index tmp: {e}"))?;
        f.sync_all().map_err(|e| format!("sync index tmp: {e}"))?;
        drop(f);
        fs::rename(&tmp, &path).map_err(|e| format!("rename index: {e}"))
    }

    fn upsert_index(&self, entry: &MemoryEntry) -> Result<(), String> {
        let mut lines = self.read_index()?.unwrap_or_default();
        let title = derive_title(entry);
        let new_line = IndexLine {
            slug: entry.slug.clone(),
            title,
            description: entry.description.clone(),
        };
        if let Some(slot) = lines.iter_mut().find(|l| l.slug == entry.slug) {
            *slot = new_line;
        } else {
            lines.push(new_line);
            lines.sort_by(|a, b| a.slug.cmp(&b.slug));
        }
        self.write_index(&lines)
    }

    fn remove_from_index(&self, slug: &str) -> Result<(), String> {
        let mut lines = self.read_index()?.unwrap_or_default();
        let before = lines.len();
        lines.retain(|l| l.slug != slug);
        if lines.len() == before {
            return Ok(());
        }
        self.write_index(&lines)
    }
}

/// One index line: the slug, the link title, and the description (mirrors the
/// frontmatter `description:` so recall and the index agree).
#[derive(Clone, Debug, PartialEq, Eq)]
struct IndexLine {
    slug: String,
    title: String,
    description: String,
}

/// `- [<Title>](<slug>.md): <description>` per entry, sorted by slug.
fn render_index(lines: &[IndexLine]) -> String {
    let mut s = String::from("# memory index — one bullet per entry.\n\n");
    for l in lines {
        s.push_str(&format!("- [{}]({}.md): {}\n", l.title, l.slug, l.description));
    }
    s
}

fn parse_index(content: &str) -> Vec<IndexLine> {
    let mut out = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        // - [Title](slug.md): description
        let Some(rest) = t.strip_prefix("- [") else {
            continue;
        };
        let Some(close_bracket) = rest.find("](") else {
            continue;
        };
        let title = rest[..close_bracket].to_string();
        let after_link = &rest[close_bracket + "](".len()..];
        let Some(close_paren) = after_link.find(')') else {
            continue;
        };
        let link = &after_link[..close_paren];
        let slug = link.strip_suffix(".md").unwrap_or(link).to_string();
        let description = after_link[close_paren + 1..]
            .trim_start_matches(':')
            .trim()
            .to_string();
        out.push(IndexLine {
            slug,
            title,
            description,
        });
    }
    out
}

/// `<cwd with every '/' replaced by '-'>` (claude-style encoding). An empty
/// cwd yields empty, which the caller should reject.
pub fn encode_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy().replace('/', "-")
}

fn derive_title(entry: &MemoryEntry) -> String {
    // The first non-empty line of the body, stripped of leading markdown
    // markers; falls back to the slug if the body is empty.
    for line in entry.body.lines() {
        let t = line.trim_start_matches('#').trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    entry.slug.clone()
}

fn yaml_escape(s: &str) -> String {
    // YAML plain scalar: quote if it contains a colon, an em-dash, or leading
    // quote; otherwise leave bare. Empty string stays empty.
    if s.is_empty() {
        return String::new();
    }
    if s.contains(':') || s.contains('"') || s.contains('—') || s.starts_with('\'') {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content.strip_prefix("---\n")?;
    let end = content.find("\n---\n")?;
    Some((&content[..end], &content[end + "\n---\n".len()..]))
}

fn parse_frontmatter(block: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in block.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        map.insert(k.trim().to_string(), v.trim().to_string());
    }
    map
}

fn parse_date(s: &str) -> Result<String, String> {
    let s = s.trim().trim_matches('"');
    if s.is_empty() {
        return Ok(crate::today());
    }
    validate_ymd(s)?;
    Ok(s.to_string())
}

fn validate_ymd(s: &str) -> Result<(), String> {
    let mut parts = s.split('-');
    let y = parts.next().ok_or("year")?;
    let m = parts.next().ok_or("month")?;
    let d = parts.next().ok_or("day")?;
    if parts.next().is_some() {
        return Err(format!("date {s:?}: extra components"));
    }
    let y: i32 = y
        .parse()
        .map_err(|_| format!("date {s:?}: year not a number"))?;
    let m: u32 = m
        .parse()
        .map_err(|_| format!("date {s:?}: month not a number"))?;
    let d: u32 = d
        .parse()
        .map_err(|_| format!("date {s:?}: day not a number"))?;
    if !(1..=9999).contains(&y) {
        return Err(format!("date {s:?}: year out of range"));
    }
    if !(1..=12).contains(&m) {
        return Err(format!("date {s:?}: month out of range"));
    }
    if !(1..=31).contains(&d) {
        return Err(format!("date {s:?}: day out of range"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_root() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "host-lifecycle-dream-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
        ));
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn encode_cwd_mangles_slashes_to_hyphens() {
        assert_eq!(
            encode_cwd(Path::new("/mnt/c/Users/dconnolly/Development/agentic-host")),
            "-mnt-c-Users-dconnolly-Development-agentic-host"
        );
    }

    #[test]
    fn entry_type_round_trips() {
        for t in [EntryType::Feedback, EntryType::Fact, EntryType::Workaround, EntryType::State] {
            assert_eq!(EntryType::parse(t.as_str()).unwrap(), t);
        }
        assert!(EntryType::parse("nonsense").is_err());
    }

    #[test]
    fn store_write_then_read_round_trips() {
        let root = tmp_root();
        let store = MemoryStore::open(&root, Path::new("/proj/agentic-host")).unwrap();
        let entry = MemoryEntry {
            slug: "github-auth".to_string(),
            description: "push as connollydavid here".to_string(),
            body: "## GitHub auth account\n\nRoute pushes through `gh auth`.\n".to_string(),
            entry_type: EntryType::Feedback,
            created: "2026-07-19".to_string(),
            last_edited: "2026-07-19".to_string(),
            superseded_by: String::new(),
        };
        store.write(&entry).unwrap();
        let read = store.read("github-auth").unwrap();
        assert_eq!(read, entry);
    }

    #[test]
    fn store_index_stays_in_sync_after_writes_and_deletes() {
        let root = tmp_root();
        let store = MemoryStore::open(&root, Path::new("/proj/x")).unwrap();
        let a = MemoryEntry {
            slug: "alpha".to_string(),
            description: "first".to_string(),
            body: "Alpha body.\n".to_string(),
            entry_type: EntryType::Fact,
            created: "2026-07-19".to_string(),
            last_edited: "2026-07-19".to_string(),
            superseded_by: String::new(),
        };
        let b = MemoryEntry {
            slug: "beta".to_string(),
            description: "second".to_string(),
            body: "Beta body.\n".to_string(),
            entry_type: EntryType::Fact,
            created: "2026-07-19".to_string(),
            last_edited: "2026-07-19".to_string(),
            superseded_by: String::new(),
        };
        store.write(&a).unwrap();
        store.write(&b).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        // Index is sorted by slug.
        assert_eq!(listed[0].slug, "alpha");
        assert_eq!(listed[1].slug, "beta");

        store.delete("alpha").unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "beta");
    }

    #[test]
    fn store_resolves_links_within_same_store_only() {
        let root = tmp_root();
        let store = MemoryStore::open(&root, Path::new("/proj/x")).unwrap();
        let entry = MemoryEntry {
            slug: "alpha".to_string(),
            description: "first".to_string(),
            body: "Alpha body. See [[beta]].\n".to_string(),
            entry_type: EntryType::Fact,
            created: "2026-07-19".to_string(),
            last_edited: "2026-07-19".to_string(),
            superseded_by: String::new(),
        };
        store.write(&entry).unwrap();
        // alpha exists; beta does not yet.
        assert!(store.resolves("alpha"));
        assert!(!store.resolves("beta"));
        let beta = MemoryEntry {
            slug: "beta".to_string(),
            description: "second".to_string(),
            body: "Beta body.\n".to_string(),
            entry_type: EntryType::Fact,
            created: "2026-07-19".to_string(),
            last_edited: "2026-07-19".to_string(),
            superseded_by: String::new(),
        };
        store.write(&beta).unwrap();
        assert!(store.resolves("beta"));
    }

    #[test]
    fn store_list_over_absent_dir_returns_empty_not_error() {
        let root = tmp_root();
        let store = MemoryStore::open(&root, Path::new("/never-written")).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(store.read("anything").is_err());
    }

    #[test]
    fn entry_render_and_parse_round_trip() {
        let entry = MemoryEntry {
            slug: "test".to_string(),
            description: "a description: with a colon".to_string(),
            body: "## Title\n\nBody text.\n".to_string(),
            entry_type: EntryType::Feedback,
            created: "2026-07-19".to_string(),
            last_edited: "2026-07-19".to_string(),
            superseded_by: "other-slug".to_string(),
        };
        let rendered = entry.render();
        let parsed = MemoryEntry::parse("test", &rendered).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn index_parse_handles_the_rendered_form() {
        let lines = vec![
            IndexLine {
                slug: "alpha".to_string(),
                title: "Alpha".to_string(),
                description: "first entry".to_string(),
            },
            IndexLine {
                slug: "beta".to_string(),
                title: "Beta".to_string(),
                description: "second: with colon".to_string(),
            },
        ];
        let rendered = render_index(&lines);
        let parsed = parse_index(&rendered);
        assert_eq!(parsed, lines);
    }

    #[test]
    fn project_encoding_is_path_independent() {
        // The same logical project reaches the same store across runs.
        let a = encode_cwd(Path::new("/mnt/c/Users/me/proj"));
        let b = encode_cwd(Path::new("/mnt/c/Users/me/proj"));
        assert_eq!(a, b);
    }
}
