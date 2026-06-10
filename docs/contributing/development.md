# Development

This page is for **contributors building CoreGraph from source**. End users do
not need any of this — they install the published CLI from npm
(`npm install -g @coregraph/cli`, see the [README](../../README.md)). Build from source
when you want to hack on the Rust workspace or the VS Code extension.

## Prerequisites

| Tool | Needed for | Notes |
|------|-----------|-------|
| Rust (stable) + Cargo | The CLI and all crates | Install via [rustup](https://rustup.rs). Edition 2021. No pinned MSRV — CI builds on stable with `clippy` and `rustfmt` components. |
| Node.js + npm | The VS Code extension only | The extension targets VS Code `^1.85.0` and builds with TypeScript `^5.3`. |
| git (+ network) | The tier-2 e2e suite only | Tier 2 shallow-clones real open-source repos at pinned SHAs. |

The Rust build needs no external indexer, language server, or compiler
toolchain for the target languages — every parser and resolver is vendored
through Cargo (see [tech-stack.md](../internals/tech-stack.md)).

## Repository layout

CoreGraph is a Cargo workspace of nine crates. `crates/cli` produces the
`coregraph` binary; everything else is a library it composes.

| Crate | Responsibility |
|-------|----------------|
| `crates/core` | Core types: `SymbolNode`, `DirectEdge`, `SymbolKind`, `EdgeKind`, ids, confidence/origin |
| `crates/manifest` | Build-system manifest parsing (Cargo, npm/pnpm/yarn, Maven, Gradle, Go, Python, Vite) |
| `crates/extractor` | tree-sitter symbol extraction per language + the documentation layer |
| `crates/stack` | stack-graphs name resolution, including the hand-authored `rules/{go,rust,kotlin}.tsg` |
| `crates/graph` | The in-memory `SymbolGraph`, indexes, snapshots, healing, cross-language mediators, risk scoring |
| `crates/query` | Query / impact / orphans / inconsistencies algorithms and output serialization |
| `crates/watcher` | File-system watching (notify) for `watch` and incremental updates |
| `crates/server` | The axum HTTP API (`handlers.rs`, `routes.rs`) |
| `crates/cli` | The `coregraph` binary: commands, daemon, IPC, and the LSP / MCP bridges |

## Build

```bash
git clone <repo-url> coregraph
cd coregraph

# Debug build — fast to compile, binary at target/debug/coregraph
cargo build --workspace

# Release build — binary at target/release/coregraph
cargo build --release
```

The `release` profile uses `lto = "fat"`, `codegen-units = 1`, and
`strip = "symbols"`: the link step is slower, but the binary is smaller and
faster. Use the debug build for day-to-day iteration and the release build for
benchmarking or the e2e suite.

Run the binary without installing it:

```bash
cargo run -p coregraph -- index --stats     # via cargo
./target/release/coregraph index --stats    # or the built binary directly
```

## Install locally

To put a source build on your `$PATH` (the from-source equivalent of
`npm install -g @coregraph/cli`):

```bash
cargo install --path crates/cli
```

This installs `coregraph` into `~/.cargo/bin`. The published npm package wraps
the same release binary produced by `cargo build --release`; developers simply
build and install it from source instead of downloading it.

To point the VS Code extension at a local build, set
`coregraph.binaryPath` to your binary (e.g. the `cargo install` path or
`target/release/coregraph`) — see [`vscode-extension/README.md`](../../vscode-extension/README.md).

## Lint, format, and test

These are the same checks CI enforces — run them before opening a PR:

```bash
cargo fmt --all -- --check                          # formatting
cargo clippy --workspace --all-targets -- -D warnings   # lint (warnings are errors)
cargo test --workspace                              # unit + integration tests
```

Some daemon-lifecycle integration tests spawn a real process and are run
single-threaded (`--test-threads=1`); see [testing.md](testing.md) for the full
test taxonomy, fixtures, and how the golden / tier-2 suites work.

End-to-end suites run from a shell script rather than `cargo test`:

```bash
tests/e2e/run.sh --golden-only   # golden scenarios — fast, no network
tests/e2e/run.sh --tier2-only    # real-repo robustness — clones projects, needs network
tests/e2e/run.sh --build         # build the workspace in release mode first
```

## VS Code extension

The extension is a thin TypeScript client over the CLI (see
[`vscode-extension/README.md`](../../vscode-extension/README.md)); it ships no graph
engine of its own.

```bash
cd vscode-extension
npm install
npm run compile          # tsc -p .         (one-off build)
npm run watch            # tsc -p . --watch  (incremental during development)

npm run test:unit        # mocha unit tests
npm run test:integration # mocha integration tests

npx vsce package                                   # produce a .vsix
code --install-extension coregraph-vscode-<version>.vsix
```

## Continuous integration

Workflows live in [`.github/workflows/`](../../.github/workflows/):

- **`ci.yml`** — on PRs and pushes to `master`/`main`: `cargo fmt --check`,
  `cargo clippy -D warnings`, `cargo build`, and `cargo test` across Ubuntu,
  macOS, and Windows; a bench compile-smoke (`cargo bench --no-run`); and a VS
  Code extension build. The Windows job tests only a subset of the workspace
  (the `core`, `graph`, `query`, and `extractor` crates plus the `cli` daemon
  unit tests and the `daemon_lifecycle` integration test); the remaining crates
  and most `cli` tests run Unix-only. See [testing.md](testing.md) for details.
- **`bench.yml`** — the criterion benchmarks.
- **`pr-review.yml`** — automated diff-impact review on pull requests.
- **`build-check.yml`** — on push/PR, compiles the release binary for every
  supported platform via `_build-matrix.yml` to catch cross-platform breakage.
- **`_build-matrix.yml`** — reusable matrix that builds the per-platform release
  binary and uploads each as a `bin-<os>-<cpu>` artifact. Called by both
  `build-check.yml` (on push/PR) and `release.yml` (when creating a GitHub
  Release). `publish-npm.yml` does not rebuild — it downloads the binaries
  already attached to the Release.
- **`release.yml`**, **`publish-npm.yml`**, **`publish-vscode.yml`** — the three
  release procedures (see [Releasing](#releasing)).

The golden and tier-2 e2e shell suites are not wired into CI; run them locally.

## Releasing

npm and the VS Code extension share **one version**: the Cargo workspace,
`crates/cli`, and `vscode-extension/package.json` must all equal the release
version before a tag is cut. Each step below is a separate `workflow_dispatch`
that runs **independently** — creating a GitHub Release does not auto-publish,
and the two publishes do not depend on each other.

1. Bump all three version sources to the new version and add a matching
   `## [<version>]` section to [`CHANGELOG.md`](../../CHANGELOG.md).
2. **Create GitHub Release** (`release.yml`) — input the version. It verifies the
   three sources agree, builds the release binary for every platform, attaches one
   archive per platform plus `SHA256SUMS` to the Release, and writes the notes from
   the changelog. It publishes nothing to npm or the Marketplace. The attached
   archives are `coregraph-<version>-<os>-<cpu>.{tar.gz,zip}`.
3. **Publish npm packages** (`publish-npm.yml`) — input the release `tag`
   (e.g. `v0.1.0`; blank resolves to the latest Release). It **downloads the
   binaries attached to that Release** (no rebuild) and publishes `@coregraph/cli`,
   so npm ships the exact bytes the Release does. Requires the Release to carry the
   per-platform assets. Defaults to a dry-run; set `dry_run=false` to ship.
4. **Publish VS Code extension** (`publish-vscode.yml`) — input the release `tag`
   (blank = latest Release). It pins the `.vsix` version to the tag and, with
   `dry_run=false` and `VSCE_PAT` set, publishes it. On dry-run / package-only
   runs (no token) the packaged `.vsix` is uploaded as an artifact; on a real
   publish `vsce publish` packages to a temp file, so no `.vsix` artifact is
   produced.

Steps 3 and 4 each run on their own and target whichever release `tag` you pass —
publish npm, the extension, or both, in any order, for any released version.

Required repository secrets: `NPM_TOKEN` (npm automation token with publish rights
to the `@coregraph` scope) and `VSCE_PAT` (VS Code Marketplace PAT). `OVSX_PAT`
is optional and enables the Open VSX mirror. Without a token, the matching
publish falls back to a dry-run / package-only run.

---

[Back to index](../README.md)
