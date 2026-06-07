# coregraph

A queryable **code symbol graph** CLI. CoreGraph combines tree-sitter (symbol
extraction) with stack-graphs (cross-file name resolution) into a single graph
you can query, export, and serve over MCP / LSP / HTTP.

> The vscode extension is still in development and untested. The CLI below is
> the supported way to use CoreGraph.

## Install

```bash
npm install -g @coregraph/cli
```

This installs the `coregraph` command. A prebuilt native binary for your
platform is pulled in automatically (via per-platform optional dependencies);
no Rust toolchain is required.

Supported platforms: `darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`,
`win32-x64`.

## Quick start

```bash
coregraph -C /path/to/project stats
coregraph -C /path/to/project query UserController
coregraph -C /path/to/project impact bootstrap
coregraph -C /path/to/project orphans
```

Run `coregraph --help` for the full command list (index, query, impact, diff,
inconsistencies, export, snapshot, server, lsp, mcp, watch, batch, plugin).

## Documentation

Full docs, architecture, and the developer build guide live in the repository:
<https://github.com/thkwag/coregraph>.

## License

MIT
