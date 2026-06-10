# Security

CoreGraph is a local developer tool. By default it reads your code, builds an
in-memory graph, and answers queries on the same machine. This page describes
what it touches, what it exposes, and the few cases where it leaves your machine.

## Threat model at a glance

| Concern | Default behavior |
|---|---|
| Network exposure | None for analysis. HTTP API is off unless you pass `--http`, and binds to `127.0.0.1` only. |
| Listening services | IPC over a per-user local socket (Unix-domain socket / Windows named pipe). No TCP port until you opt into `--http`. |
| Outbound network | None during indexing/querying. The only command that talks to the network is `coregraph review`, which shells out to the GitHub CLI (`gh`). |
| Files read | Only files under the project root you point it at, discovered through a `.gitignore`-aware walk. |
| Secrets | CoreGraph stores symbol/string values verbatim in its graph and snapshot. It does NOT redact secrets — see [Secrets and generated files](#secrets-and-generated-files). |

## Indexing is local and offline

`coregraph index` and every query run entirely on your machine:

- File discovery uses a `.gitignore`-aware directory walk rooted at the project
  you select with `-C, --project` (default `.`). Ignored paths are skipped.
- No analysis path makes outbound network calls — the symbol extractor,
  graph, and name-resolution crates have no HTTP/network dependencies.
- The graph lives in memory in the daemon process and is persisted only to a
  local binary snapshot under `<project>/.coregraph/`.

The one exception is `coregraph review --pr <N>`, which posts a PR comment.
It does this by invoking the GitHub CLI (`gh pr view`, `gh pr comment`), so it
uses your existing `gh` authentication. CoreGraph never handles a GitHub token
itself. Use `coregraph review --pr <N> --dry-run` to compute the comment
without posting anything.

## HTTP API: localhost-only by default

The HTTP API is opt-in. It only starts when you launch the daemon with `--http`:

```
coregraph server start --http
```

With no address, `--http` binds to the default `127.0.0.1:27787`. The port is
deliberately off the common 8080/8000/3000 band to avoid clashing with local
dev servers.

> The HTTP API is unauthenticated. Anything that can reach the bound address can
> read your graph. Keep it on loopback.

### The `--allow-external` gate

CoreGraph refuses to bind the HTTP API to a non-loopback address unless you
explicitly pass `--allow-external`. Loopback means `127.0.0.1`, `localhost`, or
`::1`. Attempting to bind elsewhere without the flag fails fast:

```
coregraph server start --http 0.0.0.0:27787
# Error: refusing to bind HTTP on non-loopback address '0.0.0.0:27787'
#        without `--allow-external` (the server is unauthenticated; expose on
#        127.0.0.1 / localhost / ::1 only, or pass `--allow-external` to override)
```

`--allow-external` only lifts the bind restriction. It does not add
authentication. If you expose the API beyond loopback, put it behind your own
access control (reverse proxy with auth, firewall rules, an SSH tunnel, etc.).
There is no built-in token, no TLS, and no per-request authorization.

### What the HTTP API exposes

When running, these routes serve graph data and source snippets:

| Method | Route | Returns |
|---|---|---|
| GET | `/health` | `{status, version, symbol_count}` |
| POST | `/query` | symbols matching a name |
| POST | `/batch` | results for a list of names |
| GET | `/api/query` | paginated symbol matches |
| GET | `/api/expand` | incoming/outgoing edges for a node |
| GET | `/api/impact` | reachable symbols for a symbol |
| GET | `/api/source` | a source snippet around `?file=&line=&context=` |

`/api/source` reads a file from disk and returns the requested lines. It serves
local source content to any client that can reach the bound address — another
reason to keep the bind on loopback and not expose it without your own access
controls.

## IPC socket

Thin-client commands talk to the background daemon over a local socket, not a
network port:

- Unix: a Unix-domain socket at `$XDG_RUNTIME_DIR/coregraph/server.sock`, or
  `~/.coregraph/server.sock` when `XDG_RUNTIME_DIR` is unset.
- Windows: a named pipe `\\.\pipe\coregraph-<USERNAME>`.

These live in per-user locations, so the daemon is reachable by your own user
session rather than over the network. A PID file (`server.pid`) is used for
lifecycle management (spawn/stop/status). On Unix it sits next to the socket
(under `$XDG_RUNTIME_DIR/coregraph/` or `~/.coregraph/`); on Windows, where the
named pipe has no filesystem location, it lives at
`%LOCALAPPDATA%\coregraph\server.pid` instead — still a per-user location. If you
don't want a daemon at all, pass `--no-auto-start` (or set
`COREGRAPH_NO_AUTO_START=1`) to run in-process.

## Secrets and generated files

CoreGraph indexes string literals and config keys, and stores their values in
the graph and in snapshots. It does NOT scan for or redact secrets. Treat the
graph and any snapshot under `.coregraph/` as containing whatever was in your
source — including secrets that happen to be checked in.

To keep sensitive files out of the graph, exclude them from indexing:

- Files matched by `.gitignore` are already skipped. If your secrets live in
  ignored files (the common case for `.env`, key files, etc.), they are not
  indexed.
- For paths that are tracked but you still want excluded, add them to the
  `[index] exclude` array in `<project>/.coregraph/config.toml`. It uses
  `.gitignore` pattern syntax:

```toml
[index]
exclude = [
  "secrets/**",
  "**/*.pem",
]
```

CoreGraph also automatically skips files that look minified or generated (whole
file packed into a few very long lines), and reports the skip:

```
coregraph: skipped 1 minified/generated file(s) (e.g. ./vscode-extension/media/cytoscape.min.js)
```

Because these heuristics are not a secret scanner, the safe practice is the same
as for any tool that reads your repo: don't commit secrets, and use `.gitignore`
or `[index] exclude` for anything you don't want in the graph or its snapshot.

## Path handling

`coregraph inspect <FILE:LINE>` and the `/api/source` route read files by the
path you give them, relative to the project root or as provided. There is no
sandbox that rejects paths outside the project root, so:

- Don't expose the HTTP API to untrusted clients (keep it on loopback).
- Treat `inspect` and `/api/source` as having the same filesystem reach as the
  user running the daemon.

## What CoreGraph does NOT do

To set expectations honestly:

- No secret detection or redaction. Values are stored verbatim.
- No HTTP authentication, tokens, or TLS. `--allow-external` only changes the
  bind address.
- No outbound telemetry. The only network egress is `coregraph review` via the
  GitHub CLI.

---

[Back to docs index](README.md)
