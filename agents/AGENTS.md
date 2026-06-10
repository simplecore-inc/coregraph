# Using CoreGraph from an AI coding agent

`coregraph` is a CLI that indexes a codebase into one queryable **symbol graph**
(tree-sitter + stack-graphs) and answers structural questions — who calls this, what breaks
if I change it, what is dead, where do these disagree — from the graph instead of re-reading
files. Install: `npm install -g @coregraph/cli`, then `coregraph index` once.

> Drop this file into your project (where your agent reads `AGENTS.md` — project root for
> Codex and opencode, or via Gemini's `context.fileName`). Claude Code users get the same
> guidance by installing the plugin instead (see the kit `README.md`).

## The rule

Treat **coregraph as the primary tool for structural / relational questions** — callers,
change impact and blast radius, dead code, cross-file inconsistencies, the impact of a git
diff — in preference to a raw grep/read sweep: one query resolves names across files and
costs far fewer tokens. Use your normal file tools for reading the logic inside a function
and for non-symbol content (comments, string contents, config values, prose). Always verify
a surprising "no callers" / "dead code" result with a targeted read.

```bash
coregraph query <Name> --direction incoming --edge-kind calls --depth 1   # direct callers
coregraph impact <Name> --risk                                  # what breaks if I change it
coregraph diff <base> --exclude-tests                           # impact of a git change
coregraph orphans --exclude-tests                               # dead-code candidates
```

Add `--output-format llm` to any analysis command (query, impact, diff, orphans,
inconsistencies, stats, inspect, index) to get compact, token-budgeted output for context.

## Full guidance

This file is a thin pointer. The complete guide — the decision criteria in full, the command
reference, result interpretation, false-positive filtering, and the analysis workflow — is
the **CoreGraph skill**:

<https://github.com/simplecore-inc/coregraph/blob/main/agents/coregraph/skills/coregraph/SKILL.md>

The MCP server (`coregraph mcp`) exposes five tools (`query`, `impact`, `orphans`,
`inconsistencies`, `stats`); `diff` / `inspect` / `export` / `review` and the filtering
flags are CLI-only. Per-agent setup (Claude / Codex / Gemini / opencode):
<https://github.com/simplecore-inc/coregraph/tree/main/agents>.
