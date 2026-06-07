# Roadmap

CoreGraph works end to end today: index a multi-language repo, query callers and
impact, hunt dead code, detect cross-language inconsistencies, and serve all of
it from a background daemon over CLI, MCP, LSP, or HTTP. This page records what
ships today and the short list of things that are still planned.

No timelines or effort estimates are given here — only status.

## Status — what's implemented today

### Indexing and graph

- Tree-sitter symbol extraction for 7 code languages — Rust, Java, Kotlin,
  TypeScript, JavaScript, Go, Python — plus config files (YAML / TOML / JSON →
  `ConfigKey` nodes) and Markdown (the documentation layer).
- Stack-graphs cross-file name resolution for **all 7 languages**: upstream rules
  for Java / TypeScript / JavaScript / Python, and CoreGraph's own hand-authored
  `.tsg` rules (`crates/stack/rules/{go,rust,kotlin}.tsg`) for Go / Rust / Kotlin.
  Resolution falls back to tree-sitter syntactic matching only when no binding is
  produced.
- Manifest parsing for npm/pnpm/yarn, Cargo, Gradle, Maven, Go, Python, and
  Vite/Webpack — module boundaries, internal/external dependencies, and
  `ExternalPackage` nodes.
- Confidence/trust model on every edge: 5 analysis origins, 4 trust models, and a
  `base(kind) × base(origin)` score decayed by stale evidence. See
  [confidence.md](confidence.md).
- Cross-language mediators producing `ExternallyMediated` edges: Spring DI, Spring
  config, React Router, Docker Compose, Go DI (`crates/graph/src/mediator/`).
- Documentation layer: `DocComment` and `DocSection` nodes with `Documents`,
  `Mentions`, and `DescribedIn` edges. `inconsistencies --category doc-drift`
  flags a `@param` / `:param` that names a parameter the signature no longer has.

### Query and analysis

- `query` — neighbors, callers, callees, with `--direction`, `--edge-kind`,
  `--depth`, paging, and on-demand healing.
- `impact` — transitive impact for a symbol; `--risk` adds blast-radius scoring,
  confidence-weighted impact, and affected tests.
- `orphans` — dead-code candidates, with a `[library API]` tag for public surface.
- `inconsistencies` — enum-mismatch / api-path / config-key / doc-drift detection.
- `diff` / `review` — impact of a git diff, and auto-commenting a GitHub PR.
- `inspect`, `stats` (`--breakdown`), `export` (`dot` / `cypher` / `json-graph`).

### Runtime and integrations

- Background daemon with auto-start, an LRU project cache, an idle auto-stop
  timer, and OS service registration (`server install`; launchd on macOS, systemd
  on Linux). See [architecture.md](architecture.md).
- Binary snapshots (bincode, schema v6) for fast cold start; `snapshot save|load`
  and `index --snapshot`.
- File watching with incremental rebuild (`watch`), epoch-keyed cache
  invalidation, and on-demand healing. See [change-tracking.md](change-tracking.md).
- **MCP** stdio bridge (`coregraph mcp`) exposing 5 tools: `query`, `impact`,
  `orphans`, `inconsistencies`, `stats`.
- **LSP** stdio bridge (`coregraph lsp`): definition, references, and workspace
  symbol providers.
- **HTTP** API (`server start --http`): `/health`, `/query`, `/batch`,
  `/api/query`, `/api/expand`, `/api/impact`, `/api/source`.

For how to use any of these, see [cli.md](cli.md) and
[integrations.md](integrations.md).

## Planned — not yet implemented

These are designed but not built. Do not rely on them; they are not exposed by
the current binary.

| Item | Description |
|------|-------------|
| Compound MCP tools | `explore`, `understand`, `prepareChange`, `batchGet`, `batchSearch` — higher-level MCP tools that bundle several primitive queries into one call. Today the MCP bridge exposes only the 5 primitive tools above. |
| MCP tool presets | `core` / `review` / `full` presets that would control which MCP tools are advertised to a client. No `--preset` or `mcp_preset` config key exists yet. |
| LSP hover | A `textDocument/hover` provider surfacing symbol context inline in the editor. The LSP bridge currently advertises only definition, references, and workspace-symbol. |
| HTTP live updates (SSE) | A server-sent-events / streaming endpoint pushing graph changes to connected clients. The HTTP API today is request/response only. |
| Ownership Overlay | A droppable, query-time annotation deriving ownership from git-blame with time decay. |
| Broader documentation cross-file relations | Richer doc-to-code linking beyond the current `Documents` / `Mentions` / `DescribedIn` edges, resolving references that span files. |

---

[Back to docs index](README.md)
