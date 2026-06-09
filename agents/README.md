# CoreGraph for AI coding agents

Everything needed to use CoreGraph from an AI coding agent — the Claude Code plugin and the
per-agent integration configs — in one place. All of it points at one source of guidance:
the skill at [`coregraph/skills/coregraph/SKILL.md`](coregraph/skills/coregraph/SKILL.md).

CoreGraph plugs in two ways, which compose:

1. **Guidance** — [`AGENTS.md`](AGENTS.md) tells the agent *when* to reach for CoreGraph
   (structural questions) instead of a raw grep/read sweep. Codex and opencode read
   `AGENTS.md` natively; Gemini can be pointed at it; Claude gets the same content as a skill.
2. **Tools** — `coregraph mcp` is a stdio MCP server exposing five native tools (`query`,
   `impact`, `orphans`, `inconsistencies`, `stats`). The rest of the CLI (`diff`, `inspect`,
   `export`, `review`, `stats --breakdown`) stays in the shell.

## Prerequisite (all agents)

```bash
npm install -g @coregraph/cli
coregraph --version
```

## Support matrix

| Agent | Guidance | MCP config | Setup |
|---|---|---|---|
| **Claude Code** | bundled skill | bundled `.mcp.json` | install the plugin (below) |
| **Codex** | `AGENTS.md` (project + `~/.codex/AGENTS.md`) | `~/.codex/config.toml` | [`codex/`](codex/) + `install.sh` |
| **Gemini CLI** | `AGENTS.md` via `context.fileName` | `~/.gemini/settings.json` | [`gemini/`](gemini/) |
| **opencode** | `AGENTS.md` (project + global) | `opencode.json` | [`opencode/`](opencode/) |

---

## Claude Code — plugin + marketplace

Add the marketplace by its hosted `marketplace.json` URL, then install — this downloads
**only the small catalog (no repo clone)**, and the plugin itself is sparse-fetched (only
`agents/coregraph`, not the whole source tree):

```text
/plugin marketplace add https://raw.githubusercontent.com/simplecore-inc/coregraph/main/.claude-plugin/marketplace.json
/plugin install coregraph@coregraph
```

The plugin's bundled MCP server connects automatically; verify with `/mcp` and `/help`.

> The `owner/repo` shorthand (`/plugin marketplace add simplecore-inc/coregraph`) also works,
> but it git-clones the whole source repo to read the catalog — the URL form above avoids that.
> The plugin's `source` is a `git-subdir` pointing at `agents/coregraph`, so either way only
> that directory is fetched on install.

## Codex

```bash
./codex/install.sh   # idempotent: adds the MCP server + a guidance block to ~/.codex/AGENTS.md
```

Or manually merge [`codex/config.toml`](codex/config.toml) into `~/.codex/config.toml`:

```toml
[mcp_servers.coregraph]
command = "coregraph"
args = ["mcp"]
```

(`codex mcp add coregraph -- coregraph mcp` does the same.) Then place [`AGENTS.md`](AGENTS.md)
at your project root — Codex merges `AGENTS.md` from the project root down to the working
directory, and also reads `~/.codex/AGENTS.md`.

## Gemini CLI

Merge [`gemini/settings.json`](gemini/settings.json) into `~/.gemini/settings.json` — it sets
both the MCP server and `context.fileName` so Gemini reads `AGENTS.md`:

```json
{
  "mcpServers": { "coregraph": { "command": "coregraph", "args": ["mcp"] } },
  "context": { "fileName": ["AGENTS.md", "GEMINI.md"] }
}
```

`gemini mcp add -s user coregraph coregraph mcp` registers the server alone. Use the **nested**
`context.fileName` (not the flat legacy `contextFileName`), then drop [`AGENTS.md`](AGENTS.md)
in your project root. Avoid underscores in MCP aliases — `coregraph` is fine.

## opencode

Merge [`opencode/opencode.json`](opencode/opencode.json) into your `opencode.json` (project)
or `~/.config/opencode/opencode.json` (global):

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "coregraph": { "type": "local", "command": ["coregraph", "mcp"], "enabled": true }
  }
}
```

For a local server: `type: "local"`, `command` is an **array**, the env key is
`environment` (not `env`). opencode reads `AGENTS.md` from the project root and
`~/.config/opencode/AGENTS.md` natively — drop [`AGENTS.md`](AGENTS.md) where you want it.
