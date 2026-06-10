//! Minimal MCP (Model Context Protocol) stdio bridge.
//! Exposes `tools/list` and `tools/call` for `query`, `impact`, `orphans`,
//! `inconsistencies`, and `stats`. Each call prefers the daemon's cached graph
//! over IPC (the bridge auto-spawns the daemon on startup), falling back to an
//! in-process `build_graph` only when the daemon is unavailable.

use crate::global_opts::GlobalOpts;
use clap::Args;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

#[derive(Args)]
pub struct McpArgs {}

pub fn run(_args: McpArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    // Absolute project root: the daemon rejects relative IPC project paths
    // (they would resolve against its own cwd). Every IPC client must send the
    // canonical path — see `GlobalOpts::project_root`.
    let project = globals.project_root();
    // Spin the daemon up so each tool call hits the cached graph
    // instead of running `build_graph` per invocation. Falls back to
    // in-process when spawn fails (sandbox, missing binary).
    let daemon_ready = crate::ipc::ensure_running(globals);

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: Value = serde_json::from_str(trimmed).unwrap_or(Value::Null);
        let id = parsed.get("id").cloned();
        let method = parsed.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let resp = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "coregraph", "version": env!("CARGO_PKG_VERSION")}
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"tools": tool_manifest()},
            }),
            "tools/call" => handle_tool_call(id, &parsed, &project, daemon_ready),
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("method not found: {}", method)}
            }),
        };
        writeln!(writer, "{}", resp)?;
        writer.flush()?;
    }
    Ok(())
}

fn tool_manifest() -> Value {
    json!([
        {
            "name": "query",
            "description": "Look up symbols by name across the project.",
            "inputSchema": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"]
            }
        },
        {
            "name": "impact",
            "description": "Impact analysis for a symbol: returns its transitive dependents (callers, their callers, …) up to `depth` (default 5) — the symbols that would break if it changed. Pass depth:1 for direct dependents only. The `transitive` flag only labels the output; it does not change the traversal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "transitive": {"type": "boolean", "default": false},
                    "depth": {"type": "integer", "default": 5}
                },
                "required": ["name"]
            }
        },
        {
            "name": "orphans",
            "description": "Dead-code candidates: code symbols with no incoming or outgoing edges.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "inconsistencies",
            "description": "Cross-file inconsistencies: enum-value collisions, api-path divergence, and config-key drift. (doc-drift is CLI-only: `inconsistencies --category doc-drift`.)",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "stats",
            "description": "Graph summary: symbol count and edge count.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

fn handle_tool_call(
    id: Option<Value>,
    req: &Value,
    project: &std::path::Path,
    daemon_ready: bool,
) -> Value {
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    // Prefer the daemon when it's up so consecutive `tools/call`s
    // reuse the cached graph. Falls back to the synchronous in-process
    // dispatch path on IPC failure.
    let response = if daemon_ready {
        let req_obj = crate::ipc::Request {
            method: name.clone(),
            params: arguments.clone(),
            project: project.to_path_buf(),
        };
        match crate::ipc::send(&req_obj) {
            Ok(r) if r.ok => r,
            _ => crate::dispatch::dispatch(&name, &arguments, project),
        }
    } else {
        crate::dispatch::dispatch(&name, &arguments, project)
    };
    if response.ok {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": response.body
                }]
            }
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32000, "message": response.error.unwrap_or_default()}
        })
    }
}
