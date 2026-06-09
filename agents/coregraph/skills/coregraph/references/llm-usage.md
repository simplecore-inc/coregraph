# Driving coregraph as an LLM agent

There is one effective way for an LLM to use CoreGraph: **the CLI with
`--output-format llm`.** It is complete (the whole command surface) and produces output
built for model context.

## The one way: `--output-format llm`

Every command takes `--output-format llm`, which emits compact, structured text paged
against a token budget and appends a `coregraph:budget` trailer (an HTML comment like
`<!-- coregraph:budget used=736 total=5600 remaining=4864 truncated=false -->`) reporting the
tokens used/total/remaining — so a result drops straight into context instead of pasting
whole files.

```bash
coregraph query <Name> --direction incoming --edge-kind calls --output-format llm
coregraph impact <Name> --risk --output-format llm
coregraph diff HEAD~5 --exclude-tests --output-format llm
```

Tune the size and trust of what reaches the model:

- **Token budget** — `--token-budget N` (default `8000`). Presets set budget + hop limit
  together: `--fast` (hop 1 / 2000), `--standard` (defaults), `--full` (hop 5 / 16000). The
  trailer's `total` is the *effective* budget — the advertised number scaled by a 0.7 safety
  margin (so `--token-budget 8000` → `total≈5600`), since token counts are byte-approximated.
- **Pagination** — when a response is truncated it returns an opaque cursor; pass it back
  with `--cursor <CURSOR>` for the next page.
- **Trust filtering** — `--min-confidence` drops low-trust edges before they reach the model
  (`0.7` default removes `PatternMatched`; `0.85` also removes `SyntaxMatched`, keeping
  `NameResolved` and `CompilerDerived`). Don't raise above `0.85` to "tighten" callers — real
  `NameResolved` `calls` edges sit at ~0.85 and get dropped above that, yielding an empty result.

The full command and flag surface is [`cli-reference.md`](cli-reference.md).

## MCP fast-path (optional)

If your agent has the CoreGraph MCP server connected (the Claude Code plugin registers it
automatically; other agents register `coregraph mcp` per their config), five native tools
answer the common questions without shelling out: `query`, `impact`, `orphans`,
`inconsistencies`, `stats`. The MCP surface is a **subset** — `diff`, `inspect`, `export`,
`review`, `stats --breakdown`, and the filtering flags above are CLI-only, so use the CLI
for everything beyond those five.

## Not for LLM agents

LSP (editor go-to-definition / references) and the HTTP API (local JSON service) exist for
editors and services, not for driving CoreGraph from an LLM. Their protocol details live in
the CoreGraph repository's `docs/integrations.md`.
