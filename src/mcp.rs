//! The spawned MCP surface (plan/0065, call/0042): a hand-rolled newline-delimited JSON-RPC
//! server over stdio exposing `init` and `adopt` as tools, with a server-initiated
//! `elicitation/create` for the project name. The exchange is turn-based over a single client
//! pipe, so no request-correlation map is needed: the elicitation response is the next line the
//! client sends. Async on tokio per the build directive; the loop is generic over the reader and
//! writer and takes an injected verb runner, so the protocol (the elicitation round trip included)
//! is unit-tested without stdio or a subprocess.

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// The two onboarding tools the server exposes, each with a flat input schema.
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
    if verb == "adopt" {
        if let Some(src) = args.get("source").and_then(Value::as_str) {
            if !src.trim().is_empty() {
                argv.push(src.to_string());
            }
        }
    }
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
    // Async stdio uses tokio's blocking pool (a current-thread runtime carries it), not the IO
    // reactor, so no `enable_io` is needed and the `net` feature stays out of the vendored set.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio current-thread runtime");
    let reader = BufReader::new(tokio::io::stdin());
    let writer = tokio::io::stdout();
    rt.block_on(serve(reader, writer, run_verb_subprocess));
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
        return serde_json::from_str(t).ok();
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
async fn serve<R, W, F>(reader: R, mut writer: W, run_verb: F)
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
                let result = handle_tool_call(&msg, &mut reader, &mut writer, client_elicits, &mut next_id, &run_verb).await;
                write_response(&mut writer, &id, result).await;
            }
            other => write_error(&mut writer, &id, -32601, &format!("method not found: {other}")).await,
        }
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
) -> Value
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
    F: Fn(&[String]) -> (String, i32),
{
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    let verb = params.get("name").and_then(Value::as_str).unwrap_or("");
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
        if let Some(resp) = read_message(reader).await {
            if resp.get("id").and_then(Value::as_i64) == Some(eid) {
                if let Some(name) = name_from_elicitation(&resp) {
                    let (t, c) = run_verb(&verb_argv(verb, &args, Some(&name)));
                    text = t;
                    code = c;
                }
            }
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
        assert_eq!(tools.as_array().unwrap().len(), 2);
        assert_eq!(tools[0]["name"], "init");
        assert_eq!(tools[1]["name"], "adopt");
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
        assert_eq!(verb_argv("adopt", &b, Some("bar")), vec!["adopt", "/home/user/notes", "--name", "bar"]);
        // no name and no override -> the subprocess will hit its own backstop
        assert_eq!(verb_argv("init", &json!({}), None), vec!["init"]);
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
        serve(BufReader::new(input.as_bytes()), &mut out, run).await;
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
        serve(BufReader::new(input.as_bytes()), &mut out, run).await;
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("elicitation/create"), "no elicitation without the capability");
        assert!(text.contains("name-required"), "the name-required text is returned as the result");
        assert!(text.contains("\"isError\":true") || text.contains("\"isError\": true"), "flagged as an error");
    }
}
