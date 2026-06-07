# CoreGraph Documentation

CoreGraph is a Rust CLI that builds an in-memory **code symbol graph** for
multi-language and monorepo codebases. It combines **tree-sitter** (symbol
extraction) with **stack-graphs** (cross-file name resolution), serves the graph
from a background daemon, and answers questions about callers, impact, dead code,
and cross-language inconsistencies — with every edge tagged by a confidence/trust
model so an LLM (or a human) knows how much to trust each relationship.

These pages are the reference for the `coregraph` CLI, its graph model, and its
integrations (MCP, LSP, HTTP). Start with the Getting started group, then dip
into Concepts when you want to know *why* a result looks the way it does.

## Getting started

| Page | What it covers |
|------|----------------|
| [overview.md](overview.md) | What CoreGraph is, the problem it solves, and core terminology |
| [use-cases.md](use-cases.md) | Task-oriented recipes: find callers, run impact, hunt dead code, check inconsistencies |
| [cli.md](cli.md) | Every subcommand, global options, output formats (`human` / `llm` / `json`), and the token budget |

## Concepts

| Page | What it covers |
|------|----------------|
| [graph-model.md](graph-model.md) | Symbol kinds, edge kinds, analysis origins, the four trust models, and the documentation layer |
| [confidence.md](confidence.md) | How edge confidence is computed and decayed |
| [architecture.md](architecture.md) | Daemon + thin-client design, in-memory layout, and concurrency model |
| [data-flow.md](data-flow.md) | How a file moves from disk through indexing into a query response |
| [change-tracking.md](change-tracking.md) | File watching, cold start, epochs, healing, and stale-edge handling |

## Integrations and operations

| Page | What it covers |
|------|----------------|
| [integrations.md](integrations.md) | MCP stdio bridge, LSP stdio bridge, and the HTTP API |
| [security.md](security.md) | Path-traversal guards, secret handling, and network exposure |
| [operations.md](operations.md) | Error handling, graceful degradation, and logging |

## Configuration

| Page | What it covers |
|------|----------------|
| [appendix/config-example.md](appendix/config-example.md) | Annotated `config.toml` example — every key and its default |

## Roadmap

| Page | What it covers |
|------|----------------|
| [roadmap.md](roadmap.md) | What ships today and what is still planned (not yet implemented) |

## Contributing and internals

These pages are for people **building or modifying CoreGraph**, not for using the
CLI. They live under `contributing/` (how to build, test, and evolve the project)
and `internals/` (deep implementation reference).

| Page | What it covers |
|------|----------------|
| [contributing/development.md](contributing/development.md) | Building from source, local install, and the build / lint / test workflow |
| [contributing/testing.md](contributing/testing.md) | Test levels, fixtures, the golden + tier-2 e2e suites, and CI |
| [contributing/schema-versioning.md](contributing/schema-versioning.md) | Snapshot schema versions, serde compatibility rules, and migration |
| [internals/tech-stack.md](internals/tech-stack.md) | Rust crates, key dependencies, and the per-language parsing pipeline |
| [internals/bloom-filter.md](internals/bloom-filter.md) | Bloom filter used in indexing |
| [internals/manifest-parser.md](internals/manifest-parser.md) | Per-ecosystem manifest parsing (Cargo, npm, Maven, Gradle, Go, Python) |

## Suggested reading order

If you are new, read in this order:

1. **[overview.md](overview.md)** — get the one-paragraph picture and the vocabulary.
2. **[cli.md](cli.md)** — install, run `coregraph index`, and learn the core commands.
3. **[use-cases.md](use-cases.md)** — work through real tasks against your own repo.
4. **[graph-model.md](graph-model.md)** and **[confidence.md](confidence.md)** — understand what each edge means and how much to trust it.
5. **[integrations.md](integrations.md)** — wire CoreGraph into your editor (LSP), an MCP client, or an HTTP service.
6. **[architecture.md](architecture.md)** and **[change-tracking.md](change-tracking.md)** — go deeper on the daemon, the in-memory graph, and how it stays fresh.

Contributing to CoreGraph itself? Start with **[contributing/development.md](contributing/development.md)** to build, install, and run the checks, then **[contributing/testing.md](contributing/testing.md)** and **[architecture.md](architecture.md)**.
