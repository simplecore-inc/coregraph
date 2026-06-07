# Integrations: MCP, LSP, HTTP

CoreGraph can be driven three ways besides the CLI:

- **MCP** — let an LLM agent (Claude Code, Claude Desktop) call the graph as tools.
- **LSP** — wire go-to-definition / find-references into an editor.
- **HTTP** — query the daemon over a local JSON API.

All three reuse the same in-memory graph. When the background daemon is already
running, the bridges talk to it; otherwise they build the graph in-process.

---

## MCP (Model Context Protocol)

Start a stdio MCP server with:

```bash
coregraph mcp
```

It speaks JSON-RPC and answers `initialize`, `tools/list`, and `tools/call`.

### Client config

Register CoreGraph in your MCP client. For Claude Code, add it to `.mcp.json`
at the project root; for Claude Desktop, use `claude_desktop_config.json`. The
shape is the same:

```json
{
  "mcpServers": {
    "coregraph": {
      "command": "coregraph",
      "args": ["mcp"]
    }
  }
}
```

The server runs in the client's working directory, which CoreGraph treats as the
project to index.

### Tools

There are exactly **five** tools. Names are plain (no prefix):

| Tool | Input | Returns |
|---|---|---|
| `query` | `{ "name": string }` (required) | Symbols matching `name` across the project |
| `impact` | `{ "name": string (required), "depth": integer = 5 }` | Transitive impact analysis for a symbol name |
| `orphans` | `{}` | Symbols with no incoming or outgoing edges (dead-code candidates) |
| `inconsistencies` | `{}` | Cross-enum value mismatches |
| `stats` | `{}` | Graph summary: node count, edge count, file count |

Note: the MCP `impact` tool takes `depth` (not `max-depth` — that rename applies
only to the CLI `impact` command).

Example `tools/call` for `impact`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "impact",
    "arguments": { "name": "build_router", "depth": 3 }
  }
}
```

---

## LSP

Start a stdio LSP bridge with:

```bash
coregraph lsp
```

Point your editor's LSP client at that command for the workspace. The server
advertises three capabilities:

| Capability | LSP request | What it does |
|---|---|---|
| `definitionProvider` | `textDocument/definition` | Jump to a symbol's definition |
| `referencesProvider` | `textDocument/references` | List all references to a symbol |
| `workspaceSymbolProvider` | `workspace/symbol` | Search symbols by name across the project |

There is no hover provider — definition, references, and workspace-symbol search
are the full LSP surface.

---

## HTTP API

Run the daemon with an HTTP listener:

```bash
coregraph server start --http
```

With no address, `--http` binds **`127.0.0.1:27787`** (deliberately off the
common 8080 / 8000 / 3000 ports). Pass an explicit address to override:

```bash
coregraph server start --http 127.0.0.1:9000
```

By default the listener is localhost-only. To bind a non-localhost address (for
example to reach the daemon from another machine), add `--allow-external`.

There is no SSE or websocket stream — the API is request/response only.

### Routes

| Method | Route | Params / body | Returns |
|---|---|---|---|
| GET | `/health` | — | `{ status, version, symbol_count }` |
| POST | `/query` | `{ name, limit = 50 }` | `{ name, count, symbols[] }` |
| POST | `/batch` | `{ queries: [name, …] }` | `{ results: [QueryResponse, …] }` |
| GET | `/api/query` | `?symbol=&page=0&page_size=50&budget=8000` | `{ query, matches[], pagination, budget }` |
| GET | `/api/expand` | `?node=<id>&budget=2000` | `{ node, incoming[], outgoing[], budget }` |
| GET | `/api/impact` | `?symbol=&depth=5` | `{ symbol, depth, reachable_count, edge_count, nodes[] }` |
| GET | `/api/source` | `?file=&line=1&context=5` | `{ file, target_line, context_lines, total_lines, snippet[] }` |

The HTTP `/api/impact` route takes `depth` (the CLI `--max-depth` rename does not
apply here).

### Examples

Health check:

```bash
curl http://127.0.0.1:27787/health
```

Look up a symbol by name (POST body):

```bash
curl -X POST http://127.0.0.1:27787/query \
  -H 'content-type: application/json' \
  -d '{"name": "compute_impact", "limit": 50}'
```

Impact analysis (GET query string):

```bash
curl 'http://127.0.0.1:27787/api/impact?symbol=build_router&depth=3'
```

### Symbol / edge field shape

Each symbol in an HTTP `/api/query` `matches[]` array carries `id`, `name`,
`kind`, `file`, `span_start`, `span_end` — no confidence/trust fields. The
confidence/trust fields (`confidence`, `trust`, `origin`, `trust_model`,
`stale_evidence_count`, `current_confidence`) appear only on **edge endpoints** —
`/api/expand`'s `incoming[]` / `outgoing[]` arrays — and in the CLI
`--output-format json` edge shape. Note that POST `/query` returns only
`symbols[]` as an array of name strings.

The same confidence/trust fields appear in the CLI's `--output-format json`
output, where an edge looks like this (CLI shape; the CLI labels endpoints with
`direction`, `other_id`, and `other_name`):

```json
{
  "direction": "incoming",
  "kind": "calls",
  "depth": 1,
  "other_id": 40,
  "other_name": "run",
  "confidence": 0.8549999594688416,
  "trust": "NameResolved",
  "origin": "NameResolved",
  "trust_model": "SourceEvidenced",
  "stale_evidence_count": 0,
  "current_confidence": 0.95
}
```

`confidence` is the static `base(kind) × base(origin)` value; `current_confidence`
is that value after decay from any stale evidence. See `confidence.md` for how the
two relate.

---

[Back to index](README.md)
