# Appendix B: Configuration File

CoreGraph reads a small TOML config file. There are two locations:

| Scope | Path | Holds |
|---|---|---|
| Global | `$XDG_CONFIG_HOME/coregraph/config.toml` (macOS: `~/Library/Application Support/coregraph/config.toml`) | Your machine-wide defaults |
| Project | `<project>/.coregraph/config.toml` | Per-repo overrides |

Project values override global values. The project file is **auto-created on first index** (you do not have to write it by hand), so most of the time you only edit an existing file to add an `exclude` pattern or bump a default.

## Quick start

Create the per-project file and look at the merged result:

```
$ coregraph config init --local
Initialized config at ./.coregraph/config.toml

$ coregraph config show
Global config:  ~/Library/Application Support/coregraph/config.toml
Project config: ./.coregraph/config.toml

  limits.token_budget            = 8000          [project]
    # Default token budget for LLM output
  limits.hop_limit               = 3             [project]
    # Default graph traversal depth
  limits.min_confidence          = 0.7           [project]
    # Default minimum edge confidence (matches clap default)
  server.max_loaded_projects     = 5             [project]
    # Maximum projects held in the daemon cache (LRU eviction above this)
  server.graceful_shutdown_sec   = 30            [project]
    # Seconds the daemon waits for in-flight queries before hard-exit on SIGTERM
  server.idle_unload_minutes     = 10            [project]
    # Minutes a project sits idle before its graph is unloaded from the daemon cache
  server.max_loaded_bytes        = 0             [project]
    # Approx. heap budget (bytes) across all loaded graphs; LRU-evicts over it. 0 = unlimited
```

The `[project]` / `[global]` / `[default]` tag after each value tells you where the effective value came from.

`config show` also lints the project file: a TOML parse failure, an unknown
section, or an unknown key inside a known section is reported as a `⚠` line
directly under the `Project config:` header (the same diagnostics are printed
as `[coregraph] WARNING:` lines on stderr by `coregraph index`). Unknown
entries are warned about, never rejected — a newer config keeps working with
an older binary.

```
Project config: ./.coregraph/config.toml
  ⚠ unknown config section [indx] in ./.coregraph/config.toml (known: limits, index, analysis, server, inconsistencies)
```

Related commands:

| Command | Effect |
|---|---|
| `coregraph config init --local` | Write `<project>/.coregraph/config.toml` (use `--force` to overwrite) |
| `coregraph config init` | Write the global config instead |
| `coregraph config show` | Print the merged (defaults + global + project) view |
| `coregraph config path` | Print both config file paths |
| `coregraph config unset <key>` | Remove a key from the global config |
| `coregraph config recommend [--write]` | Suggest values for the keys below, measured from the indexed graph |

You don't have to tune these keys by hand: `coregraph config recommend`
derives suggested values for `index.exclude`, `index.string_match_max_files`,
`analysis.exclude`, and `inconsistencies.disable` from the indexed graph
(data-dominated files, projected string-pair volume, same-language api-path
noise, generated-file content markers) and prints them with one reason per
suggestion. `--write` merges the suggestions into the project file —
comment-preserving, appending arrays without duplicates. See
[`config recommend` in the CLI reference](../cli.md#config-recommend).

## The generated file

This is exactly what `coregraph config init --local` writes. Only these keys exist — there are no language, manifest, or healing sections.

```toml
# CoreGraph configuration
#
# [limits] — query defaults applied when the matching CLI flag
#   is not explicitly passed. Override per-command with
#   `--token-budget`, `--hop-limit`, or `--min-confidence`.
#
# [index]  — indexing-time knobs.
#   exclude: gitignore-syntax patterns for files NOT parsed at
#            all (no symbols, no edges). Cuts symbols/memory, but
#            dropping a file also drops the edges it would have
#            contributed — so a symbol referenced ONLY by an
#            excluded file becomes a false orphan. Example:
#            ["tests/fixtures/", "target/"]
#   string_match_max_files: skip cross-file StringMatch pairing for a
#            string value found in more than N distinct files (convention
#            strings would otherwise emit O(k²) edges). 0 = unlimited.
#
# [analysis] — analysis-surface knobs.
#   exclude: gitignore-syntax patterns for files still PARSED
#            (their edges keep referenced symbols connected) but
#            whose own symbols are hidden from dead-code (orphans)
#            reports. Prefer this over index.exclude for generated
#            consumers like routeTree.gen.ts. Example: ["**/*.gen.ts"]
#
# [inconsistencies] — cross-file detector tuning.
#   disable: categories to skip when running without --category
#            (e.g. ["api-path"] for a frontend-only repo where
#            route literals have no server counterpart).
#   api_path_min_segments: minimum non-empty path segments for an
#            api-path near-miss report (default 2).
#
# Keys:
#   limits.token_budget        Default token budget for LLM output
#   limits.hop_limit           Default graph traversal depth
#   limits.min_confidence      Default minimum edge confidence (matches clap default)
#   server.max_loaded_projects Maximum projects held in the daemon cache (LRU eviction above this)
#   server.graceful_shutdown_sec Seconds the daemon waits for in-flight queries before hard-exit on SIGTERM
#   server.idle_unload_minutes Minutes a project sits idle before its graph is unloaded from the daemon cache
#   server.max_loaded_bytes    Approx. heap budget (bytes) across all loaded graphs; LRU-evicts over it. 0 = unlimited
#   index.exclude              Gitignore patterns for files excluded from indexing (array)
#   analysis.exclude           Gitignore patterns for files kept indexed but hidden from
#                              dead-code reports (array)

[analysis]
exclude = []

[inconsistencies]
api_path_min_segments = 2
disable = []

[index]
exclude = []
string_match_max_files = 8

[limits]
hop_limit = 3
min_confidence = 0.7
token_budget = 8000

[server]
graceful_shutdown_sec = 30
idle_unload_minutes = 10
max_loaded_bytes = 0
max_loaded_projects = 5
```

## Key reference

| Key | Default | Meaning |
|---|---|---|
| `limits.token_budget` | `8000` | Default token budget for LLM output. Overridden by `--token-budget`. |
| `limits.hop_limit` | `3` | Default graph traversal depth. Overridden by `--hop-limit`. |
| `limits.min_confidence` | `0.7` | Default minimum edge confidence to include (range `0.0`–`1.0`). Overridden by `--min-confidence`. |
| `server.max_loaded_projects` | `5` | Maximum projects held in the daemon cache; the LRU project is evicted above this. |
| `server.graceful_shutdown_sec` | `30` | Seconds the daemon waits for in-flight queries before a hard exit on `SIGTERM`. |
| `server.idle_unload_minutes` | `10` | Minutes a project sits idle before its graph is unloaded from the daemon cache. |
| `server.max_loaded_bytes` | `0` | Approximate heap budget in bytes across all loaded graphs; the LRU project is evicted above it. `0` = unlimited. |
| `index.exclude` | `[]` | Gitignore-syntax patterns for files excluded from indexing entirely (array). |
| `index.string_match_max_files` | `8` | Hub cap for cross-file `StringMatch` pairing: a string value found in more than this many distinct files is skipped (convention strings would otherwise emit O(k²) edges). `0` = unlimited. |
| `analysis.exclude` | `[]` | Gitignore-syntax patterns for files that stay indexed but whose own symbols are hidden from dead-code (orphans) reports (array). |
| `inconsistencies.disable` | `[]` | Categories (`"enum-mismatch"`, `"api-path"`, `"config-key"`) skipped when `inconsistencies` runs without `--category`. An explicit `--category X` always runs X regardless of this list. Unknown labels warn and are ignored. |
| `inconsistencies.api_path_min_segments` | `2` | Minimum non-empty `/`-separated segments BOTH paths need before an api-path near-miss is reported (`/auth` vs `/auths` is a route-name convention, not a typo). `0` disables the floor; a negative value warns and is treated as `0`. |

The three `[limits]` keys set the per-query defaults: when you run a command without the matching CLI flag, the config value is used. An explicit flag on the command line always wins over the config file.

The four `[server]` keys are read when a daemon starts (project-local values
over global ones) and seed its project-cache lifecycle; the
`--auto-stop-minutes` CLI flag still wins where it overlaps.

## `[index] exclude` — skipping paths

This is the key you will edit most. It is an array of gitignore-syntax patterns. Patterns you add here are applied **on top of** a built-in set that is always active, so you only list what is specific to your repo.

```toml
[index]
exclude = [
  "tests/fixtures/",   # don't treat fixture code as real symbols
  "generated/",        # generated output you don't want in analysis
  "!generated/keep/",  # ...but un-ignore one subtree with a leading !
]
```

Always-on built-in exclusions (you do not need to list these):

| Category | Patterns |
|---|---|
| VCS / IDE | `.git/`, `.idea/`, `.vscode/` |
| Build output | `target/`, `build/`, `dist/`, `out/` (and `**/` nested variants) |
| Dependency caches | `node_modules/`, `.gradle/`, `vendor/`, `__pycache__/`, `.venv/`, `venv/` (and `**/` nested variants) |

Notes:

- Full gitignore syntax is supported, including the `!` negation prefix to re-include a subtree of an otherwise-excluded directory.
- A directory pattern such as `tests/fixtures/` also excludes everything nested below it.
- The built-in patterns also match nested occurrences in monorepos (e.g. `apps/foo/target/`), so a single config covers a multi-package tree.
- A malformed pattern is **skipped with a stderr warning**
  (`[coregraph] WARNING: invalid exclude pattern '…' skipped`); the other
  patterns — including the built-ins — stay active. One bad glob never
  disables the matcher. A TOML parse failure of the whole file likewise warns
  and falls back to defaults instead of failing silently.
- After `coregraph index`, files whose config-key / string-literal / doc-section
  symbols dominate the graph are listed in a `note:` block as candidates for
  this list (human output only; suggestion-only — nothing is excluded
  automatically).

---
[Back to index](../README.md)
