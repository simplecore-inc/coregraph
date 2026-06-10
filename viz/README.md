# coregraph atlas

A 3D viewer for coregraph symbol graphs (React 19, Vite 8, react-force-graph-3d /
three.js). The production build is one self-contained `dist/index.html`,
embedded into the `coregraph` binary at build time.

## Official: `coregraph viz`

```sh
coregraph viz                 # serves http://127.0.0.1:7321/, opens the browser
```

One command, no Node required at runtime. The server talks to the coregraph
daemon over its IPC socket:

- If the daemon is running, the connect screen lists every project currently
  loaded in daemon memory — pick one to view it instantly.
- If the daemon is stopped (or the project is not loaded yet), enter a project
  path: the daemon is started automatically, the project is indexed, and the
  graph is streamed straight out of daemon memory (`export_graph` method).
- Progress is shown as staged feedback: connect → index/extract → build.

Options: `--port N`, `--no-open`, `--html <file>` (serve a dev build instead
of the embedded viewer). The server binds to 127.0.0.1 only and guards every
API call with a per-process token (CSRF/DNS-rebinding protection).

Building the binary with the viewer embedded:

```sh
cd viz && npm install && npm run build   # → dist/index.html
cargo build -p coregraph                 # build.rs embeds it
```

Without the npm step the binary still builds; `coregraph viz` then serves a
placeholder page that explains how to bundle the viewer.

## Development: Node bridge

```sh
npm install
npm run build          # → dist/index.html
npm run serve          # node server.mjs — same API as `coregraph viz`
```

Useful when iterating on the viewer without rebuilding the Rust binary
(combine with `coregraph viz --html dist/index.html` or plain `npm run serve`).
Options: `--port N` (or `COREGRAPH_VIZ_PORT`), `--bin /path/to/coregraph`
(or `COREGRAPH_BIN`), `--no-open`.

## Offline mode

The same `dist/index.html` opened from `file://` (or any static host without
the bridge) accepts drag & drop of a json-graph export:

```sh
coregraph -C <project> export --format json-graph > graph.json
```

To bake a dataset into the HTML instead, run `npm run data` (or pipe any
export through `node scripts/embed-data.mjs`) before `npm run build`.

## Usage

- **`/`** focuses the search bar; picking a result isolates that symbol's
  neighborhood (depth 1–3, configurable).
- **Click** a node to inspect it: kind, file, source snippet, and incident
  edges grouped by kind and direction. Clicking a neighbor walks the graph.
- **Esc** exits path mode, then isolate mode, then clears the selection.
- Left panel: symbol-kind / edge-kind / analysis-origin filters, minimum edge
  confidence, min-degree / hide-hubs filters, modules overview (collapse by
  directory; click a supernode to drill in), color by kind or directory,
  switch project, forced re-index.
- **Analysis** (live mode): impact of the selected symbol (blast-radius
  gradient + risk score + affected tests), dead code overlay, cross-file
  inconsistencies (pair edges), git diff impact (`base ref` input), and
  shortest path between two symbols (client-side).
- Search-bar actions: share link (`#v=` hash restores project, symbol and
  filters), PNG capture, visible-subgraph JSON download.
- The viewer polls the daemon every 5s; when the project's graph changes on
  the daemon (file watcher / re-index), a banner offers a one-click reload —
  it never auto-reloads under you.
- Drag & drop any json-graph export onto the window at any time.

Structural kinds (File, DocComment, StringLiteral, ConfigKey) and plumbing
edges (Contains, BelongsTo, Resolves, Documents) are hidden by default to keep
the code-symbol view readable; every kind can be re-enabled in the panel.

WebGL is required. Node sizes follow degree; edge opacity dims outside the
selected neighborhood.

```sh
npm test               # vitest unit tests for the data layer
```
