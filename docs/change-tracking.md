# Change tracking, healing, and freshness

CoreGraph keeps its symbol graph in sync with your source as files change. You
get fresh answers without paying a full re-index on every edit. This page shows
how to run the watcher, what "on-demand healing" does for queries, and the
freshness model (epochs and stale nodes) underneath.

## Watch a project

`coregraph watch` builds the graph once, then rebuilds incrementally as files
change. Press Ctrl+C to stop.

```
$ coregraph watch
Watching: /path/to/project (press Ctrl+C to stop)
Initial index: 3396 symbols, 21342 edges
Change detected in 1 relevant file(s) — updating...
  incremental: -8 symbols from changed files, +9 re-extracted (epoch 1)
Rebuilt: 3397 symbols, 21345 edges
```

By default each change triggers an **incremental** update: the changed files are
invalidated and re-extracted, and only their slice of the graph is rebuilt. The
graph epoch advances by one each cycle (see [Epochs](#epochs-and-freshness)).

> The initial-index line is real output; the incremental delta lines (the
> per-cycle `+`/`-` counts, the epoch value, and the post-rebuild totals) are
> illustrative — they depend on the edit you make and cannot be reproduced
> without simulating one. The line format matches the actual output.

### Watch flags

| Flag | Effect |
|------|--------|
| `--diff` | Print the per-symbol delta (`+`/`-`) instead of just new totals |
| `--no-incremental` | Rebuild the whole graph on every change instead of the incremental invalidate+heal path |

With `--diff` you see exactly which symbols appeared or disappeared:

```
$ coregraph watch --diff
Watching: /path/to/project (press Ctrl+C to stop)
Initial index: 3396 symbols, 21342 edges
Change detected in 1 relevant file(s) — updating...
  incremental: -1 symbols from changed files, +2 re-extracted (epoch 1)
  + parse_header [Function] @ crates/extractor/src/incremental.rs
  - parse_hdr [Function] @ crates/extractor/src/incremental.rs
```

(The `+`/`-` lines here illustrate a symbol rename in one file; the exact symbols
and counts depend on the edit you make.)

`--no-incremental` forces a from-scratch rebuild each time. It is slower but
useful as a correctness baseline if you suspect an incremental update produced a
wrong graph.

> The background daemon runs the same incremental loop internally. You usually do
> not need to run `watch` yourself — thin-client commands (`query`, `impact`, …)
> auto-spawn the daemon, which watches the project and heals on demand. Run
> `watch` directly when you want to see change events in your own terminal.

## How change detection works

Change tracking runs in three layers. Each layer filters out work the next layer
would otherwise waste time on.

```
Layer 1  File change detection   notify events + content hash
Layer 2  AST change detection    tree-sitter re-extracts the changed files
Layer 3  Graph propagation       evidence-based invalidation (no cascade)
```

### Layer 1 — file change detection

The watcher receives OS file-system events through the `notify` crate. Two things
make those events reliable:

- **Debounce.** Events are coalesced in a short window (100 ms) before they reach
  the rebuild logic. This merges the temp-write-then-rename bursts that editors
  like VS Code and IntelliJ produce on save into a single change.
- **Content hashing.** A file-system event does not mean the bytes changed. Every
  candidate path is re-hashed (xxh3-64) and compared against the last-known hash;
  if the hash matches, the event is dropped. Format-on-save that rewrites
  identical content, or a `touch`, never triggers a rebuild.

```
$ coregraph watch
...
No content changes detected (hash match) — skipping.
```

Files larger than 10 MiB are not hashed; the watcher falls back to mtime+size for
those. Build outputs, dependency caches, and your configured `exclude` patterns
are filtered out at the watcher boundary, so `target/`, `node_modules/`, and
friends never wake the rebuild loop.

### Layer 2 — AST change detection

For each file that really changed, CoreGraph re-runs the language extractor
(tree-sitter parse + symbol extraction). Only the changed files are re-parsed;
unchanged files keep their existing nodes and edges.

### Layer 3 — graph propagation (evidence-based)

Every node and edge in the graph remembers the **evidence file** it came from —
the source file whose parse produced it. When a file changes, only the graph
content evidenced by that file is invalidated:

- Nodes defined in the file are marked `Stale`, then replaced by the re-extracted
  definitions.
- Edges whose evidence file is the changed file are dropped and re-created from
  the new parse.
- Edges that point *into* the file from elsewhere are left alone — their evidence
  is the *source* side, which did not change.

Because invalidation is keyed on the evidence (source) side of each edge, **there
is no stale cascade.** Editing one file does not force a re-evaluation of every
file that references it.

When a file is deleted, its nodes are marked `Gone` rather than removed
immediately. Incoming structural edges survive as tombstones so callers still see
the historical shape, and a periodic GC pass reaps `Gone` nodes after a 5-minute
grace period.

Node lifecycle states:

| Status | Meaning |
|--------|---------|
| `Verified` | Definition matches current source |
| `Stale` | Source file changed; healing has not yet confirmed the node's fate |
| `Assumed` | Node had no direct evidence (inferred / cross-file fixup) |
| `Gone` | Source deleted or definition vanished on re-parse; pending GC |

## On-demand healing (query time)

You do not have to keep a watcher running to get fresh query results. When a
daemon-routed read command (`query`, `impact`, `inconsistencies`, `diff`) hits
the daemon, it first checks whether any file in the queried graph has a changed
content hash, and re-extracts the stale ones **before** answering — under a
**200 ms budget**.

- Files that re-extract within the budget are healed in place; the query runs on
  the fresh graph.
- If healing runs past 200 ms, the remaining files are left stale, the query
  proceeds on the pre-heal graph, and the response carries a warning:

```
⚠ healing in progress for 2 file(s)
```

This keeps queries point-in-time-correct for the region you actually asked about,
without ever blocking a query behind a full re-index.

### Skipping healing

Pass `--no-heal` to skip the freshness check and query the cached graph directly.
Use it when you want the lowest latency and are fine with a slightly stale answer
(for example, batch tooling that re-indexes on its own schedule):

```
$ coregraph query compute_impact --no-heal
```

Healing applies to the `query` path only. The daemon's internal freshness-update
operation (its `reindex` IPC dispatch method — not a CLI subcommand) is itself a
rebuild, so it skips healing entirely. The user-facing command that reindexes a
project is `index`.

## Epochs and freshness

The graph carries a monotonic **epoch** counter (`GraphEpoch`, a `u64` starting
at 0). It increments by one on every invalidate+heal cycle — you can see it in the
`watch` output (`epoch 1`, `epoch 2`, …).

The epoch exists so cached query results can be invalidated precisely. A cache
entry is keyed by `(query, epoch)`; once a file change bumps the epoch, old
entries are naturally missed and evicted. There is no TTL timer — cache
invalidation is driven only by real evidence changes, so it stays exactly in sync
with the graph.

## Git-aware diff

Most projects are git repos, and CoreGraph uses git where it helps. The `diff`
command computes which files changed between two revisions using git directly:

```
$ coregraph diff HEAD~1 --exclude-tests
Diff HEAD~1..HEAD: 52 file(s), 974 touched symbol(s), 1659 reachable (depth 3)
  • reindex_latency.rs [File] @ crates/cli/examples/reindex_latency.rs
  • main [Function] @ crates/cli/examples/reindex_latency.rs
  ...
  … and 954 more
```

Under the hood:

- **Committed changes** between revisions come from `git diff --name-only <from>..<to>`.
- **Working-tree edits** (uncommitted changes) are added via `git diff --name-only HEAD`
  when the target is `HEAD`, so "what does my branch touch" includes work in
  progress.

The watcher is also git-aware in one important way: if a multi-file git operation
is in progress (a merge, rebase, or cherry-pick — detected via `MERGE_HEAD`,
`REBASE_HEAD`, or `CHERRY_PICK_HEAD` in `.git/`), it skips the rebuild for that
batch. The intermediate state mid-operation is unreliable, and git fires another
event when the operation completes.

```
$ coregraph watch
...
Git operation in progress, skipping rebuild
```

## Snapshots

A snapshot is a bincode binary blob of the graph (nodes, edges, epoch, and a
schema version). It is a portable, on-disk copy of the graph you can write out
and inspect.

```
$ coregraph snapshot save --out graph.bin
$ coregraph snapshot load graph.bin
```

- `snapshot save --out <PATH>` and `index --snapshot <PATH>` both write a
  snapshot file. `save_snapshot` serializes the graph (a magic header, a u32
  schema version, then the bincode body) and writes it to the target path.
- `snapshot load <PATH>` deserializes a snapshot and prints a one-line summary
  (epoch, plus node and edge counts).

A corrupt or schema-mismatched snapshot produces a clear error: the loader
validates the magic bytes and the schema version *before* bincode touches the
body, so you get a clean message instead of a deserialization stack trace. When
that happens, reindex with `coregraph index --snapshot`. See
[schema-versioning.md](contributing/schema-versioning.md) for the snapshot format and version
history.

---

[Back to index](README.md)
