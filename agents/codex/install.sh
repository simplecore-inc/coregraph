#!/usr/bin/env bash
# Install CoreGraph for OpenAI Codex CLI: register the MCP server in ~/.codex/config.toml
# and add a guidance block to ~/.codex/AGENTS.md. Idempotent — safe to re-run.
set -euo pipefail

CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
CONFIG_TOML="$CODEX_HOME/config.toml"
AGENTS_MD="$CODEX_HOME/AGENTS.md"
AGENTS_URL="https://github.com/simplecore-inc/coregraph/blob/main/agents/AGENTS.md"

mkdir -p "$CODEX_HOME"

if ! command -v coregraph >/dev/null 2>&1; then
  echo "⚠ coregraph is not on PATH. Install it first:" >&2
  echo "    npm install -g @coregraph/cli" >&2
  echo "  Continuing — config will be written, but the MCP server won't start until coregraph is installed." >&2
fi

# 1. MCP server registration ------------------------------------------------
if [ -f "$CONFIG_TOML" ] && grep -q '^\[mcp_servers\.coregraph\]' "$CONFIG_TOML"; then
  echo "✔ [mcp_servers.coregraph] already present in $CONFIG_TOML — skipping."
else
  {
    printf '\n[mcp_servers.coregraph]\n'
    printf 'command = "coregraph"\n'
    printf 'args = ["mcp"]\n'
  } >> "$CONFIG_TOML"
  echo "✔ Added [mcp_servers.coregraph] to $CONFIG_TOML"
fi

# 2. Guidance block ---------------------------------------------------------
MARKER_START="<!-- coregraph:start -->"
MARKER_END="<!-- coregraph:end -->"
if [ -f "$AGENTS_MD" ] && grep -qF "$MARKER_START" "$AGENTS_MD"; then
  echo "✔ CoreGraph guidance already present in $AGENTS_MD — skipping."
else
  cat >> "$AGENTS_MD" <<EOF

$MARKER_START
## CoreGraph

The \`coregraph\` CLI/MCP indexes a codebase into a queryable symbol graph. Prefer it as the
PRIMARY tool for **structural / relational** questions — who calls X, change impact and
blast radius, dead code / orphans, cross-file inconsistencies, the impact of a git diff —
in preference to a raw grep/read sweep: one query resolves names across files and costs far
fewer tokens. Use normal file tools for reading the logic inside a function and for
non-symbol content (comments, string contents, config values, prose). Verify a surprising
"no callers" / "dead code" result with a targeted read.

Setup: \`coregraph index\` once, then query. MCP tools (\`query\`, \`impact\`, \`orphans\`,
\`inconsistencies\`, \`stats\`) cover the common cases; \`diff\` / \`inspect\` / \`export\` /
\`review\` are CLI-only. Full guide: $AGENTS_URL
$MARKER_END
EOF
  echo "✔ Added CoreGraph guidance to $AGENTS_MD"
fi

echo
echo "Done. Start a new Codex session and run /mcp to confirm the coregraph server is connected."
