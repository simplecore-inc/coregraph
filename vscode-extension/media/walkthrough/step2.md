# Understanding Scores

CoreGraph attaches five metrics to every symbol in the call graph.

## Score glossary

| Score | Meaning |
|---|---|
| **reach** | Number of distinct symbols that can transitively call this symbol. Higher reach = more callers. |
| **edges** | Direct call-graph edges touching this symbol (callers + callees combined). |
| **impact** | Confidence-weighted sum of reach across all callers. A proxy for "how much of the codebase depends on this". |
| **confidence** | Fraction of calls to this symbol whose type is statically resolved (0–100 %). Lower confidence means the graph is partially inferred. |
| **stale** | Marked when the last indexed version of the symbol differs from the on-disk source. Re-save the file to clear. |
| **Orphan** | A symbol with no callers and no callees — potentially dead code. |

## When to act

- **High impact (> 20 by default)**: refactor carefully. Any change here propagates widely.
- **Low confidence**: the call graph may be incomplete; treat impact numbers as lower bounds.
- **Stale**: scores reflect an older graph snapshot. Save the file to trigger a fast reindex.
- **Orphan**: consider whether the symbol is genuinely unused before deleting it.

> The warn threshold defaults to **20** (`coregraph.warnOnCommit.impactThreshold`). Adjust it in Settings.
