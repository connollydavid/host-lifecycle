//! The spawned MCP surface (plan/0065, call/0042): a hand-rolled newline-delimited JSON-RPC
//! server over stdio exposing `init` and `adopt` as tools, with a server-initiated
//! `elicitation/create` for the project name. The exchange is turn-based over a single client
//! pipe, so no request-correlation map is needed: the elicitation response is the next line the
//! client sends. Async on tokio per the build directive; the loop is generic over the reader and
//! writer and takes an injected verb runner, so the protocol (the elicitation round trip included)
//! is unit-tested without stdio or a subprocess.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::memory::{EntryType, MemoryEntry, MemoryStore};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// The onboarding + memory tools the server exposes. The onboarding tools
/// (plan/0065) run as subprocesses; the memory tools (plan/0073) call the
/// memory store directly.
fn tool_defs() -> Value {
    json!([
        {
            "name": "init",
            "description": "Create a fresh agentic project named agentic-<name> in a new folder.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "the project name (a lowercase slug)"},
                    "purpose": {"type": "string", "description": "an optional one-line purpose"},
                    "at": {"type": "string", "description": "an optional parent directory (default the working directory)"}
                }
            }
        },
        {
            "name": "adopt",
            "description": "Bring a folder under the methodology: refuse a software repo, adopt an empty agentic-<name> in place, else create the host elsewhere from an arbitrary folder.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {"type": "string", "description": "the folder to adopt (default the working directory)"},
                    "name": {"type": "string", "description": "the project name for the create-elsewhere route (a lowercase slug)"},
                    "purpose": {"type": "string", "description": "an optional one-line purpose"},
                    "at": {"type": "string", "description": "an optional parent directory for the new host"}
                }
            }
        },
        {
            "name": "memory_list",
            "description": "List every memory entry in the per-user host-* memory store for the current project.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "memory_read",
            "description": "Read one memory entry by slug from the per-user store.",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": {"type": "string", "description": "the entry slug"}},
                "required": ["slug"]
            }
        },
        {
            "name": "memory_write",
            "description": "Create, update, or delete a memory entry in the per-user store. The repo MEMORY.md (append-only) is never writable via this tool.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "description": "the entry slug"},
                    "description": {"type": "string", "description": "a one-line summary (what recall keys on)"},
                    "body": {"type": "string", "description": "the free-form markdown body"},
                    "type": {"type": "string", "enum": ["feedback", "fact", "workaround", "state"], "description": "the entry class (default fact)"},
                    "superseded_by": {"type": "string", "description": "a slug that supersedes this entry"},
                    "op": {"type": "string", "enum": ["write", "delete"], "description": "write (create-or-update) or delete (default write)"}
                },
                "required": ["slug"]
            }
        },
        {
            "name": "memory_consolidate",
            "description": "Run the dream audit over both memory stores (repo MEMORY.md + per-user) and return the findings. Read-only; the advisory memory-consolidation pass.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

/// The `initialize` result: echo the protocol version and advertise the tools capability.
fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {"tools": {}},
        "serverInfo": {
            "name": "host-lifecycle",
            "title": "host-lifecycle onboarding",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

/// The server-initiated `elicitation/create` request asking for the project name (a flat
/// object of one string property, per the elicitation schema restriction).
fn elicitation_request(id: i64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "elicitation/create",
        "params": {
            "message": "Project name for the new agentic host (a lowercase slug):",
            "requestedSchema": {
                "type": "object",
                "properties": {"name": {"type": "string", "title": "Project name"}},
                "required": ["name"]
            }
        }
    })
}

/// Whether the client declared the `elicitation` capability in its initialize params.
fn client_has_elicitation(init_params: &Value) -> bool {
    init_params
        .get("capabilities")
        .and_then(|c| c.get("elicitation"))
        .is_some()
}

/// The name from an elicitation response, if the client accepted with a non-empty one. A
/// decline or a cancel (or an accept with no name) yields `None`.
fn name_from_elicitation(resp: &Value) -> Option<String> {
    let result = resp.get("result")?;
    if result.get("action").and_then(Value::as_str)? != "accept" {
        return None;
    }
    let name = result.get("content")?.get("name")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// A text tool result (`content` array), flagged as an error when `is_error`.
fn tool_result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

/// Build the argv for a verb subprocess from the tool-call arguments. `adopt` passes the
/// source folder positionally; both map name (an override wins), purpose, and at.
fn verb_argv(verb: &str, args: &Value, name_override: Option<&str>) -> Vec<String> {
    let mut argv = vec![verb.to_string()];
    let name = name_override
        .or_else(|| args.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(n) = name {
        argv.push("--name".into());
        argv.push(n.to_string());
    }
    for (flag, key) in [("--purpose", "purpose"), ("--at", "at")] {
        if let Some(v) = args.get(key).and_then(Value::as_str) {
            if !v.trim().is_empty() {
                argv.push(flag.into());
                argv.push(v.to_string());
            }
        }
    }
    // The adopt source follows an end-of-options `--`, so a `source` value beginning with `-` cannot
    // be read as a flag by the subprocess. This closes argument injection from the MCP source field.
    if verb == "adopt" {
        if let Some(src) = args.get("source").and_then(Value::as_str) {
            if !src.trim().is_empty() {
                argv.push("--".into());
                argv.push(src.to_string());
            }
        }
    }
    argv
}

/// Run a verb as a subprocess of this same binary, returning (combined output, exit code). The
/// verb writes the CLI handoff to stdout; a failure appends its stderr so the tool result carries it.
fn run_verb_subprocess(argv: &[String]) -> (String, i32) {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("host-lifecycle"));
    match std::process::Command::new(exe).args(argv).output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            if !o.status.success() {
                text.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            (text, o.status.code().unwrap_or(-1))
        }
        Err(e) => (format!("could not run `{}`: {e}", argv.join(" ")), -1),
    }
}

/// The public entry: build a current-thread tokio runtime and serve over real stdio.
pub fn mcp(_args: &[String]) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio current-thread runtime");
    let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let reader = BufReader::new(tokio::io::stdin());
    let writer = tokio::io::stdout();
    rt.block_on(serve(reader, writer, run_verb_subprocess, project_dir));
}

/// Read one newline-delimited JSON message, skipping blank lines. `None` on a closed pipe.
async fn read_message<R: AsyncBufRead + Unpin>(reader: &mut R) -> Option<Value> {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        match serde_json::from_str(t) {
            Ok(v) => return Some(v),
            // A non-JSON line is logged-and-ignored per the spec, not end-of-input, so skip it
            // rather than conflating a parse failure with a closed pipe (which would kill the server).
            Err(_) => continue,
        }
    }
}

async fn write_value<W: AsyncWrite + Unpin>(writer: &mut W, v: &Value) {
    let mut s = v.to_string();
    s.push('\n');
    let _ = writer.write_all(s.as_bytes()).await;
    let _ = writer.flush().await;
}

async fn write_response<W: AsyncWrite + Unpin>(writer: &mut W, id: &Value, result: Value) {
    if id.is_null() {
        return; // a notification gets no response
    }
    write_value(writer, &json!({"jsonrpc": "2.0", "id": id, "result": result})).await;
}

async fn write_error<W: AsyncWrite + Unpin>(writer: &mut W, id: &Value, code: i64, message: &str) {
    write_value(writer, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})).await;
}

/// The protocol loop, generic over reader/writer and the verb runner. Reads newline-delimited
/// JSON-RPC and handles `initialize`, `tools/list`, and `tools/call` (eliciting the name when the
/// client declared the capability and the verb reports name-required) until the client closes stdin.
async fn serve<R, W, F>(reader: R, mut writer: W, run_verb: F, project_dir: PathBuf)
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
    F: Fn(&[String]) -> (String, i32),
{
    let mut reader = reader;
    let mut client_elicits = false;
    let mut next_id: i64 = 1;
    loop {
        let Some(msg) = read_message(&mut reader).await else { break };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => {
                if let Some(params) = msg.get("params") {
                    client_elicits = client_has_elicitation(params);
                }
                write_response(&mut writer, &id, initialize_result()).await;
            }
            "notifications/initialized" | "notifications/cancelled" | "" => { /* no reply */ }
            "ping" => write_response(&mut writer, &id, json!({})).await,
            "tools/list" => write_response(&mut writer, &id, json!({"tools": tool_defs()})).await,
            "tools/call" => {
                let result = handle_tool_call(&msg, &mut reader, &mut writer, client_elicits, &mut next_id, &run_verb, &project_dir).await;
                write_response(&mut writer, &id, result).await;
            }
            other => write_error(&mut writer, &id, -32601, &format!("method not found: {other}")).await,
        }
    }
}

/// Handle a `tools/call` for a memory tool (plan/0073). These call the memory
/// store directly (no subprocess); the per-user store at
/// `~/.host-memory/<encoded-cwd>/` is the only writable target. The repo
/// MEMORY.md is never writable via MCP.
fn handle_memory_call(verb: &str, args: &Value, project_dir: &Path) -> Value {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(home) = home else {
        return tool_result("no HOME set; cannot locate the per-user memory store", true);
    };
    let root = home.join(".host-memory");
    let store = match MemoryStore::open(&root, project_dir) {
        Ok(s) => s,
        Err(e) => return tool_result(&format!("cannot open memory store: {e}"), true),
    };
    match verb {
        "memory_list" => {
            let entries = match store.list() {
                Ok(e) => e,
                Err(e) => return tool_result(&format!("list: {e}"), true),
            };
            if entries.is_empty() {
                return tool_result("(no memory entries)", false);
            }
            let text = entries
                .iter()
                .map(|e| format!("- [{}]({}.md): {}", e.slug, e.slug, e.description))
                .collect::<Vec<_>>()
                .join("\n");
            tool_result(&text, false)
        }
        "memory_read" => {
            let slug = args.get("slug").and_then(Value::as_str).unwrap_or("").trim();
            if slug.is_empty() {
                return tool_result("memory_read requires a 'slug'", true);
            }
            match store.read(slug) {
                Ok(entry) => tool_result(&entry.render(), false),
                Err(e) => tool_result(&format!("read {slug}: {e}"), true),
            }
        }
        "memory_write" => {
            let slug = args.get("slug").and_then(Value::as_str).unwrap_or("").trim();
            if slug.is_empty() {
                return tool_result("memory_write requires a 'slug'", true);
            }
            let op = args.get("op").and_then(Value::as_str).unwrap_or("write");
            if op == "delete" {
                return match store.delete(slug) {
                    Ok(()) => tool_result(&format!("deleted {slug}"), false),
                    Err(e) => tool_result(&format!("delete {slug}: {e}"), true),
                };
            }
            let description = args.get("description").and_then(Value::as_str).unwrap_or("").to_string();
            let body = args.get("body").and_then(Value::as_str).unwrap_or("").to_string();
            let entry_type = args
                .get("type")
                .and_then(Value::as_str)
                .and_then(|s| EntryType::parse(s).ok())
                .unwrap_or(EntryType::Fact);
            let superseded_by = args
                .get("superseded_by")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let today = crate::today();
            let created = store.read(slug).map(|e| e.created).unwrap_or_else(|_| today.clone());
            let entry = MemoryEntry {
                slug: slug.to_string(),
                description,
                body,
                entry_type,
                created,
                last_edited: today,
                superseded_by,
            };
            match store.write(&entry) {
                Ok(()) => tool_result(&format!("wrote {slug}"), false),
                Err(e) => tool_result(&format!("write {slug}: {e}"), true),
            }
        }
        "memory_consolidate" => {
            let audit = crate::dream::run_audit(project_dir);
            let coverage = crate::dream::coverage_lines(&audit.marker, audit.store_present, audit.history_checked)
                .join("\n");
            let findings = audit.findings;
            if findings.is_empty() {
                tool_result(&format!("clean\n{coverage}"), false)
            } else {
                let lines: Vec<String> = findings
                    .iter()
                    .map(|f| {
                        format!(
                            "{} ({}) [{}] {} route={}: {}\n  → {}",
                            f.entry_slug,
                            match f.store {
                                crate::dream::StoreLoc::Repo => "repo",
                                crate::dream::StoreLoc::PerUser => "per-user",
                            },
                            f.kind,
                            f.confidence.as_str(),
                            match f.route {
                                crate::dream::Route::Edit => "edit",
                                crate::dream::Route::Append => "append",
                            },
                            f.explanation,
                            f.suggestion
                        )
                    })
                    .collect();
                let confirmed = findings
                    .iter()
                    .filter(|f| f.confidence == crate::dream::Confidence::Confirmed)
                    .count();
                tool_result(
                    &format!(
                        "{} confirmed finding(s), {} review prompt(s):\n{}\n{}",
                        confirmed,
                        findings.len() - confirmed,
                        lines.join("\n"),
                        coverage
                    ),
                    false,
                )
            }
        }
        _ => tool_result(&format!("unknown memory tool: {verb}"), true),
    }
}

/// Handle a `tools/call`: run the verb subprocess; if it reports name-required (the exit-code
/// backstop) and the client can elicit and none was given, elicit the name and re-run with it.
async fn handle_tool_call<R, W, F>(
    msg: &Value,
    reader: &mut R,
    writer: &mut W,
    client_elicits: bool,
    next_id: &mut i64,
    run_verb: &F,
    project_dir: &Path,
) -> Value
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
    F: Fn(&[String]) -> (String, i32),
{
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    let verb = params.get("name").and_then(Value::as_str).unwrap_or("");

    // Memory tools (plan/0073): direct Rust calls to the memory store; no subprocess.
    if matches!(verb, "memory_list" | "memory_read" | "memory_write" | "memory_consolidate") {
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        return handle_memory_call(verb, &args, project_dir);
    }

    if verb != "init" && verb != "adopt" {
        return tool_result(&format!("unknown tool: {verb}"), true);
    }
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let name_given = args
        .get("name")
        .and_then(Value::as_str)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    let (mut text, mut code) = run_verb(&verb_argv(verb, &args, None));
    if code == crate::EXIT_NAME_REQUIRED && client_elicits && !name_given {
        let eid = *next_id;
        *next_id += 1;
        write_value(writer, &elicitation_request(eid)).await;
        // Read until the response for our elicitation id arrives, skipping any interleaved
        // notification or unrelated message rather than assuming it is the very next line.
        loop {
            let Some(resp) = read_message(reader).await else { break };
            if resp.get("id").and_then(Value::as_i64) != Some(eid) {
                continue; // a notification or stray message; keep waiting for our response
            }
            if let Some(name) = name_from_elicitation(&resp) {
                let (t, c) = run_verb(&verb_argv(verb, &args, Some(&name)));
                text = t;
                code = c;
            }
            break;
        }
    }
    tool_result(&text, code != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_json_shapes_are_correct() {
        assert_eq!(initialize_result()["protocolVersion"], PROTOCOL_VERSION);
        assert!(initialize_result()["capabilities"]["tools"].is_object());
        let tools = tool_defs();
        assert_eq!(tools.as_array().unwrap().len(), 6);
        assert_eq!(tools[0]["name"], "init");
        assert_eq!(tools[1]["name"], "adopt");
        assert_eq!(tools[2]["name"], "memory_list");
        assert_eq!(tools[3]["name"], "memory_read");
        assert_eq!(tools[4]["name"], "memory_write");
        assert_eq!(tools[5]["name"], "memory_consolidate");
        let req = elicitation_request(7);
        assert_eq!(req["method"], "elicitation/create");
        assert_eq!(req["id"], 7);
        assert_eq!(req["params"]["requestedSchema"]["properties"]["name"]["type"], "string");
    }

    #[test]
    fn client_capability_and_elicitation_response_parsing() {
        assert!(client_has_elicitation(&json!({"capabilities": {"elicitation": {}}})));
        assert!(!client_has_elicitation(&json!({"capabilities": {"roots": {}}})));
        assert_eq!(
            name_from_elicitation(&json!({"result": {"action": "accept", "content": {"name": " acme "}}})).as_deref(),
            Some("acme")
        );
        assert_eq!(name_from_elicitation(&json!({"result": {"action": "decline"}})), None);
        assert_eq!(name_from_elicitation(&json!({"result": {"action": "cancel"}})), None);
    }

    #[test]
    fn verb_argv_maps_arguments() {
        let a = json!({"purpose": "reader", "at": "/tmp"});
        assert_eq!(verb_argv("init", &a, Some("acme")), vec!["init", "--name", "acme", "--purpose", "reader", "--at", "/tmp"]);
        let b = json!({"source": "/home/user/notes"});
        assert_eq!(verb_argv("adopt", &b, Some("bar")), vec!["adopt", "--name", "bar", "--", "/home/user/notes"]);
        // a source beginning with `-` is guarded behind `--`, not read as a flag by the subprocess
        let inj = json!({"source": "--at"});
        assert_eq!(verb_argv("adopt", &inj, None), vec!["adopt", "--", "--at"]);
        // no name and no override -> the subprocess will hit its own backstop
        assert_eq!(verb_argv("init", &json!({}), None), vec!["init"]);
    }

    // F2: an interleaved notification between elicitation/create and the response must not be
    // mistaken for the response; the server skips it and still gets the name.
    #[tokio::test]
    async fn serve_skips_an_interleaved_notification_during_elicitation() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{"elicitation":{}}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"init","arguments":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":9}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":1,"result":{"action":"accept","content":{"name":"acme"}}}"#, "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        let run = |argv: &[String]| {
            if argv.iter().any(|a| a == "--name") { (format!("ok {}", argv.join(" ")), 0) }
            else { ("name-required\n".to_string(), crate::EXIT_NAME_REQUIRED) }
        };
        serve(BufReader::new(input.as_bytes()), &mut out, run, std::path::PathBuf::from(".")).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("--name acme"), "the interleaved notification was skipped and the elicited name resolved");
    }

    // The full turn-based flow with elicitation: init is called with no name, the stub verb
    // reports name-required (exit 3), the server elicits, the client accepts "acme", and the
    // re-run with --name succeeds. A stub verb models the exit-code backstop.
    #[tokio::test]
    async fn serve_elicits_the_name_then_reruns() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{"elicitation":{}}}}"#, "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"init","arguments":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":1,"result":{"action":"accept","content":{"name":"acme"}}}"#, "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        let run = |argv: &[String]| {
            if argv.iter().any(|a| a == "--name") {
                (format!("host-path: ./agentic-acme\nargv: {}\n", argv.join(" ")), 0)
            } else {
                ("name-required: supply the project name\n".to_string(), crate::EXIT_NAME_REQUIRED)
            }
        };
        serve(BufReader::new(input.as_bytes()), &mut out, run, std::path::PathBuf::from(".")).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(PROTOCOL_VERSION), "initialize echoed the protocol version");
        assert!(text.contains("\"name\":\"init\"") || text.contains("\"name\": \"init\""), "tools/list carried init");
        assert!(text.contains("elicitation/create"), "the server elicited the name");
        assert!(text.contains("host-path: ./agentic-acme"), "the re-run handoff is the tool result");
        assert!(text.contains("--name acme"), "the re-run passed the elicited name");
    }

    // Without the elicitation capability, a nameless call returns the name-required text as an
    // error result (no elicitation), so the agent re-calls with a name argument (the backstop).
    #[tokio::test]
    async fn serve_returns_name_required_without_elicitation() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"init","arguments":{}}}"#, "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        let run = |argv: &[String]| {
            if argv.iter().any(|a| a == "--name") {
                ("host-path: x\n".to_string(), 0)
            } else {
                ("name-required: supply the project name\n".to_string(), crate::EXIT_NAME_REQUIRED)
            }
        };
        serve(BufReader::new(input.as_bytes()), &mut out, run, std::path::PathBuf::from(".")).await;
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("elicitation/create"), "no elicitation without the capability");
        assert!(text.contains("name-required"), "the name-required text is returned as the result");
        assert!(text.contains("\"isError\":true") || text.contains("\"isError\": true"), "flagged as an error");
    }

    // --- Memory tool tests (plan/0073 #extend-mcp) ---
    // These drive the MCP server over stdio JSON-RPC with a real per-user
    // memory store fixture. HOME is set to a tmp dir so the store opens there.

    fn memory_fixture() -> (std::path::PathBuf, std::path::PathBuf) {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let home = std::env::temp_dir().join(format!("mcp-mem-home-{n}"));
        let project = std::env::temp_dir().join(format!("mcp-mem-proj-{n}"));
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&project);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(home.join(".host-memory")).unwrap();
        (home, project)
    }

    #[tokio::test]
    async fn memory_write_then_list_then_read_round_trips() {
        let (home, project) = memory_fixture();
        std::env::set_var("HOME", home.as_os_str());
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_write","arguments":{"slug":"alpha","description":"first","body":"Alpha body."}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"memory_list","arguments":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"memory_read","arguments":{"slug":"alpha"}}}"#, "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        let run = |_: &[String]| ("unused\n".to_string(), 0);
        serve(
            BufReader::new(input.as_bytes()),
            &mut out,
            run,
            project.clone(),
        )
        .await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("wrote alpha"), "write result: {text}");
        assert!(text.contains("alpha.md"), "list result includes the entry: {text}");
        assert!(text.contains("Alpha body"), "read result includes the body: {text}");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[tokio::test]
    async fn memory_consolidate_reports_findings() {
        let (home, project) = memory_fixture();
        std::env::set_var("HOME", home.as_os_str());
        // Write a repo MEMORY.md with a room reference so consolidate finds it.
        std::fs::write(
            project.join("MEMORY.md"),
            "# M\n\n### Entry\n\nCites call/0017.\n",
        )
        .unwrap();
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_consolidate","arguments":{}}}"#, "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        let run = |_: &[String]| ("unused\n".to_string(), 0);
        serve(
            BufReader::new(input.as_bytes()),
            &mut out,
            run,
            project.clone(),
        )
        .await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("room-touching"), "consolidate found the call/ ref: {text}");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[tokio::test]
    async fn memory_write_delete_round_trips() {
        let (home, project) = memory_fixture();
        std::env::set_var("HOME", home.as_os_str());
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_write","arguments":{"slug":"temp","description":"temp","body":"Temp."}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"memory_write","arguments":{"slug":"temp","op":"delete"}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"memory_list","arguments":{}}}"#, "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        let run = |_: &[String]| ("unused\n".to_string(), 0);
        serve(
            BufReader::new(input.as_bytes()),
            &mut out,
            run,
            project.clone(),
        )
        .await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("deleted temp"), "delete result: {text}");
        assert!(text.contains("no memory entries"), "list after delete is empty: {text}");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[tokio::test]
    async fn init_and_adopt_still_pass_unchanged() {
        // The plan/0065 onboarding tools are byte-identical; the new project_dir
        // parameter does not affect their dispatch.
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"init","arguments":{"name":"acme"}}}"#, "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        let run = |argv: &[String]| {
            if argv.iter().any(|a| a == "--name" && argv[argv.len() - 1] == "acme") {
                ("host-path: ./agentic-acme\n".to_string(), 0)
            } else {
                ("error\n".to_string(), 1)
            }
        };
        serve(BufReader::new(input.as_bytes()), &mut out, run, std::path::PathBuf::from(".")).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("host-path: ./agentic-acme"), "init still works with the new signature");
    }
}
