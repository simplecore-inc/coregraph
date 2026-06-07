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
```

The `[project]` / `[global]` / `[default]` tag after each value tells you where the effective value came from.

Related commands:

| Command | Effect |
|---|---|
| `coregraph config init --local` | Write `<project>/.coregraph/config.toml` (use `--force` to overwrite) |
| `coregraph config init` | Write the global config instead |
| `coregraph config show` | Print the merged (defaults + global + project) view |
| `coregraph config path` | Print both config file paths |
| `coregraph config unset <key>` | Remove a key from the global config |

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
#   exclude: gitignore-syntax patterns for paths that analysis
#            commands (orphans, impact, inconsistencies, …) should
#            skip. Example: ["tests/fixtures/", "target/"]
#
# Keys:
#   limits.token_budget        Default token budget for LLM output
#   limits.hop_limit           Default graph traversal depth
#   limits.min_confidence      Default minimum edge confidence (matches clap default)
#   server.max_loaded_projects Maximum projects held in the daemon cache (LRU eviction above this)
#   server.graceful_shutdown_sec Seconds the daemon waits for in-flight queries before hard-exit on SIGTERM
#   index.exclude              Gitignore patterns for analysis exclusions (array)

[index]
exclude = []

[limits]
hop_limit = 3
min_confidence = 0.7
token_budget = 8000

[server]
graceful_shutdown_sec = 30
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
| `index.exclude` | `[]` | Gitignore-syntax patterns for paths that analysis commands should skip (array). |

The three `[limits]` keys set the per-query defaults: when you run a command without the matching CLI flag, the config value is used. An explicit flag on the command line always wins over the config file.

> **Note:** The two `[server]` keys (`server.max_loaded_projects`, `server.graceful_shutdown_sec`) are written by `config init` and shown by `config show`, but are **not yet wired to the daemon**. Editing them has no runtime effect: the LRU cache cap is currently fixed at **5** and the graceful-shutdown grace period at **30s**.

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

---
[Back to index](../README.md)
