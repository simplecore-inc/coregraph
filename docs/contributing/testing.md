# Testing

A contributor's guide to how CoreGraph is tested and how to run each layer
locally. There are three layers: Rust `cargo test` (unit + integration),
shell-driven end-to-end **golden** scenarios, and a real-repo **tier 2**
robustness smoke test. Benchmarks run separately under `criterion`.

## Run everything

```sh
# 1. Rust unit + integration tests across the workspace.
cargo test --workspace

# 2. Build the binary the shell tests need, then run the golden scenarios.
cargo build --workspace
bash tests/e2e/golden/run.sh

# 3. Real-repo smoke test (clones pinned open-source projects).
cargo build --release            # tier 2 prefers the release binary
bash tests/e2e/tier2/run.sh

# 4. Benchmarks (optional, never gates a PR).
cargo bench --workspace
```

The golden and tier 2 runners are plain Bash. They require the `coregraph`
binary to be built first and use `jq` for JSON assertions — install `jq` if you
do not have it.

## Test layers

| Layer | Lives in | Run with | What it checks |
|---|---|---|---|
| Unit + integration | `crates/*/src/` and `crates/*/tests/` | `cargo test --workspace` | Per-crate logic, parsers, confidence math, daemon lifecycle |
| E2E golden | `tests/e2e/golden/` | `bash tests/e2e/golden/run.sh` | Exact CLI behavior on small hand-built fixtures (precision) |
| E2E tier 2 | `tests/e2e/tier2/` | `bash tests/e2e/tier2/run.sh` | Binary survives large real repos without collapse |
| Benchmarks | `crates/extractor/benches/`, `crates/query/benches/` | `cargo bench --workspace` | Indexing / impact timing under `criterion` |

## Unit and integration tests

Standard Rust tests: `#[test]` and `#[tokio::test]` functions live inline in
`crates/*/src/` modules and in dedicated files under `crates/*/tests/`. Tests
that need a workspace on disk create an isolated temp directory with `tempfile`.

```sh
cargo test --workspace                       # all crates
cargo test -p coregraph-graph                # one crate
cargo test -p coregraph --test daemon_lifecycle -- --test-threads=1
```

Integration-test files worth knowing:

| File | Covers |
|---|---|
| `crates/cli/tests/daemon_lifecycle.rs` | Daemon spawn / reuse / stop over the IPC socket (run single-threaded) |
| `crates/extractor/tests/cross_language_fixtures.rs` | Cross-language linking: ApiPathMatch edges connecting a Java controller and a TS client (against the `cross-lang-matched` fixture) |
| `crates/stack/tests/stack_graphs_backend.rs` | The stack-graphs cross-file name resolver |
| `crates/stack/tests/go_tsg.rs`, `rust_tsg.rs`, `kotlin_tsg.rs` | The hand-authored `.tsg` rules for Go, Rust, and Kotlin |

Snapshot serialization is covered by ordinary unit tests in
`crates/graph/src/snapshot.rs`: a node/edge round-trip
(`snapshot_roundtrip_nodes_and_edges`), an empty-graph round-trip
(`empty_graph_snapshot`), and a guard that loading a snapshot written under an
older schema version is rejected so the daemon falls back to a full rebuild
(`load_rejects_wrong_schema_version`). See `schema-versioning.md` for the schema
version (`v6`) and the forced-rebuild path.

## E2E golden scenarios

The golden suite is the precision layer. Each scenario is a self-contained
directory under `tests/e2e/golden/` with a small inline source fixture (typically
under `src/`, or split by package/language for the cross-package and multi-language
scenarios) and a `scenario.sh` that indexes it and asserts exact counts and symbol
names. `run.sh` executes every scenario in sequence, killing any leftover daemon
between them so cached state from one fixture cannot bleed into the next.

| Scenario | Exercises |
|---|---|
| `01-ts-single-package` | `stats`, `query`, `export`, `impact`, `--output-format json` on one TS package |
| `02-ts-cross-package` | Cross-package name resolution in a TS monorepo |
| `03-orphans` | `orphans` dead-code detection |
| `04-inconsistencies` | Cross-enum / api-path / config-key detection |
| `05-multi-lang` | Go + Rust + TypeScript in one project |
| `06-python` | The Python extractor |
| `07-edge-cases` | Barrels, circular imports, dynamic patterns, generics |
| `08-cli-protocols` | `batch`, `snapshot save`/`load`, `watch`, `config show` |

Shared shell helpers live in `tests/e2e/lib/`:

- `common.sh` — locates the binary (defaults to `target/debug/coregraph`),
  provides `run_cg <project_dir> <subcommand> …`, and the assertion helpers
  (`assert_eq`, `assert_range`, `assert_contains`, `assert_jq`, `print_summary`).
- `invariants.sh` — structural invariants reused across scenarios.

A scenario asserts concrete facts, not approximate ones. For example,
`01-ts-single-package` checks that `export --format json-graph` yields a specific
node count and that `--min-confidence 0.0` widens the edge set, and that named
symbols (`UserService`, `getUser`, …) are present while a known-absent symbol is
not. To run or debug a single scenario:

```sh
bash tests/e2e/golden/01-ts-single-package/scenario.sh
```

## E2E tier 2 (real repositories)

Tier 2 is the robustness layer. It shallow-clones pinned open-source projects and
runs `stats`, `query <well-known-symbol>`, and `orphans` against each, asserting
only that the binary exits 0, produces non-zero output, stays within wide
symbol/edge ranges, and finishes inside a wall-clock limit. It is
collapse-prevention, not precision — a regression that wipes out edges (for
example a broken resolver) drops below `min_edges` and fails; the golden suite is
where exact correctness is enforced.

Projects are declared in `tests/e2e/tier2/projects.toml`, each pinned to a commit
SHA for reproducibility:

| Project | Why it is in the set |
|---|---|
| `zustand` | Small TypeScript library |
| `excalidraw` | Large TypeScript codebase |
| `spring-petclinic` | Java with both Maven and Gradle, plus `application.yml` config keys |
| `requests` | Python library |
| `cobra` | Go CLI library |
| `exposed` | Kotlin (exercises the hand-authored `kotlin.tsg` rules) |

Each `[[project]]` entry sets `min_symbols` / `max_symbols`, `min_edges` /
`max_edges`, `max_wall_clock_sec`, and a `well_known_symbols` list that must
resolve. The ranges are deliberately wide — bump the SHA and the ranges together
when you intentionally change extraction. Clones are cached under
`tests/e2e/tier2/cache/`, and each run writes a JSON snapshot to
`tests/e2e/tier2/snapshots/<project>-<sha>.json` for regression tracking. Tier 2
prefers the release binary and warns (then runs slowly) if only a debug build is
present.

## Benchmarks

Benchmarks use `criterion` and live in `crates/extractor/benches/build_graph.rs`
(graph construction) and `crates/query/benches/impact.rs` (impact traversal).

```sh
cargo bench --workspace
```

Benchmarks never gate a pull request. In CI, the `ci.yml` `bench-smoke` job only
compiles them (`cargo bench --workspace --no-run`) to catch breakage, and the
separate `bench.yml` workflow runs them and posts a comment if a result
regresses past its threshold — it does not fail the build.

## CI

The `ci.yml` workflow runs on every push to `master`/`main` and on pull requests,
across Linux, macOS, and Windows:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo build --workspace`
4. `cargo test --workspace` on Unix. Windows runs the library crates plus the
   daemon module and the `daemon_lifecycle` integration test (single-threaded),
   because spawning a real daemon under privileged process control is Unix-only
   for now.

The shell-driven golden and tier 2 suites are not wired into CI — run them
locally with `bash tests/e2e/golden/run.sh` and `bash tests/e2e/tier2/run.sh`.
Only the Rust tests, a bench compile-smoke, and the VS Code extension build run
in `ci.yml`; the criterion benchmarks run in the separate `bench.yml` workflow.

## Writing a new test

- **A new CLI behavior or a precision regression you want pinned** → add a golden
  scenario (or extend an existing one) with a minimal inline fixture and exact
  assertions. Reuse the helpers in `tests/e2e/lib/common.sh`.
- **Crate-internal logic** → a `#[test]` next to the code, or a file under that
  crate's `tests/` for integration-level coverage. Use `tempfile` for any
  on-disk workspace so tests stay isolated.
- **A new language or extractor** → add a fixture under `tests/fixtures/` (see the
  existing per-language fixtures such as `java-spring`, `typescript-react`, `go`,
  `python-simple`, `kotlin-simple`, `rust-simple`, and `cross-lang-matched`) and
  assert against it from `crates/extractor/tests/`.

---
[Back to index](../README.md)
