# CoreGraph: Overview

CoreGraph builds an in-memory **code symbol graph** for multi-language and
monorepo codebases. It combines **tree-sitter** (symbol extraction) with
**stack-graphs** (cross-file name resolution), serves the graph from a
background daemon, and answers questions a single-file search cannot — who calls
this, what breaks if I change it, what is dead, and where do two languages
disagree about the same value.

## What it answers

Ask "who calls `compute_impact`?" and you get the callers, not text matches:

```
$ coregraph query compute_impact --direction incoming --edge-kind calls --hop-limit 1

── query: compute_impact ──────────────────────────────────

✓ compute_impact [crates/query/src/impact.rs:27]
  kind: Function | package: query (cargo)

  Incoming (14):
  ├── calls ← run [Function] @ crates/cli/src/commands/diff.rs      [0.85] ✓
  ├── calls ← run [Function] @ crates/cli/src/commands/impact.rs      [0.85] ✓
  ├── calls ← cached_impact [Function] @ crates/cli/src/dispatch.rs      [0.85] ✓
  ├── calls ← api_impact [Function] @ crates/server/src/handlers.rs      [0.85] ✓
  └── ... (14 total)
  ✓ trust: all paths verified

── page 1/1 | 14 edges total | budget: 506/5600 tokens ──
```

Each edge carries a **confidence score** (`[0.85]`) and a trust mark (`✓`), so an
LLM — or a human — knows how much weight to give the relationship. That trust
tagging is the core idea; everything else builds on it.

## Why it exists

Existing tools each miss something:

| Tool | Limitation |
|---|---|
| `grep` / regex | No syntax awareness. Can't follow renames, re-exports, or dynamic dispatch; matches text, not symbols. |
| `ctags` | Indexes definition locations only. No reference edges, no value-level tracking. |
| Single-language LSP | Sees one language at a time. Can't connect cross-language references or wire config to code. |

CoreGraph pairs tree-sitter parsing with stack-graphs name resolution to build a
project-wide graph that spans languages, then exposes queries tuned for feeding
context to an LLM.

## Identity: a graph builder, not a compiler

CoreGraph is deliberately **not** a compiler.

- A compiler's consumer is a machine, and a 0.01% error fails the build.
- CoreGraph's consumer is an LLM (or a person), which reasons usefully from
  partial information.
- Dropping a real relationship because it couldn't be proven is more harmful than
  surfacing it with a lower confidence score.

So CoreGraph prefers **trust-tagging plus fast answers** over compiler-grade
certainty. Every edge is annotated with how it was derived and how much to trust
it (see [Confidence](confidence.md)), instead of being silently discarded.

## What you can do that `grep` can't

| Capability | What it does | Try |
|---|---|---|
| Dead-code detection | Lists public symbols with no semantic edges in either direction (no callers and no callees) — accounting for renames, re-exports, and dynamic dispatch. Structural edges (Contains/BelongsTo/Documents/DescribedIn) are excluded from this count. | `coregraph orphans` |
| Cross-language linking | Connects code across language boundaries through dedicated mediators (Spring DI/config, React Router, Docker Compose, Go DI) and shared API paths (`ApiPathMatch` edges). | `coregraph query <symbol>` |
| Config ↔ code consistency | Relates config keys (`application.yml`, `docker-compose.yml`, …) to the code that reads them. | `coregraph inconsistencies --category config-key` |
| Enum / value mismatches | Flags the same logical value spelled differently across languages (e.g. a Java enum constant vs. a TypeScript string). | `coregraph inconsistencies --category enum-mismatch` |
| Impact analysis | Computes the transitive set a change can reach, with optional risk scoring and affected tests. | `coregraph impact <symbol> --risk` |
| LLM context shaping | Extracts an N-hop subgraph around a symbol and serializes it to fit a token budget. | `coregraph query <symbol> --depth 2` |

CoreGraph also tracks documentation: `///` / `/** */` doc comments and Markdown
sections become nodes, so `inconsistencies --category doc-drift` can catch a
`@param` that names an argument the signature no longer has.

All seven code languages — Rust, Java, Kotlin, TypeScript, JavaScript, Go,
Python — have stack-graphs name-resolution rules (Java/TS/JS/Python from
upstream, Go/Rust/Kotlin hand-authored here), so cross-file resolution works the
same way across the stack.

## Glossary

- **Evidence** — the source file that justifies an edge. A `calls` edge's
  evidence is the file containing the call site. When that file changes, the edge
  becomes stale.
- **Stale** — a symbol or edge is stale when its source file changed and the
  extracted data is no longer current.
- **Healing** — re-parsing stale files to refresh the graph. CoreGraph heals
  on-demand at query time: a query re-extracts the files it touches before
  answering. Pass `--no-heal` to skip it and read the graph as-is.
- **Epoch** — an immutable version number for the graph. Reads consume the
  current epoch without locking; a write swaps in a new epoch atomically. See
  [Architecture](architecture.md) for the concurrency model.
- **Server (daemon)** — a single background process that holds the in-memory
  graph for one or more projects. It serves the IPC socket and the HTTP API, and
  backs the LSP and MCP stdio bridges (which reuse its in-memory graph when
  running).
- **Client (thin client)** — the CLI. It forwards queries to the daemon over an
  IPC socket and auto-starts the daemon on first use (or runs in-process with
  `--no-auto-start`).
- **ACTIVE / UNLOADED** — a project's load state in the daemon. ACTIVE means its
  graph is in memory; UNLOADED means it was snapshotted to disk and dropped from
  memory after sitting idle. See [Architecture](architecture.md).

---
[Back to index](README.md)
