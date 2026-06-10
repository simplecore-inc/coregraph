#!/usr/bin/env bash
# Local end-to-end verification of the npm packaging for the HOST platform only.
#
# It builds the release binary, assembles the host platform package + the main
# package, packs both as tarballs, installs them into a throwaway consumer
# project, and runs the installed `coregraph` launcher to prove:
#   1. --version          (launcher resolves & execs the native binary)
#   2. a real query        (the binary actually analyzes a fixture)
#   3. an MCP stdio round-trip (the binary serves protocol over stdio)
#
# The platform tarball is installed alongside the main tarball because neither
# is on the registry yet; in production the platform package is pulled via the
# main package's optionalDependencies. Nothing here publishes.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

HOST_OS="$(node -e 'process.stdout.write(process.platform)')"
HOST_CPU="$(node -e 'process.stdout.write(process.arch)')"
echo "── host platform: ${HOST_OS}-${HOST_CPU}"

# 1. release binary --------------------------------------------------------
BIN="$REPO_ROOT/target/release/coregraph"
if [[ ! -x "$BIN" ]]; then
  echo "── building release binary (cargo build --release -p coregraph) ..."
  cargo build --release -p coregraph
fi
echo "── native binary: $("$BIN" --version)"

# 2. assemble packages -----------------------------------------------------
node npm/scripts/build-platform.mjs --os "$HOST_OS" --cpu "$HOST_CPU" --binary "$BIN"
node npm/scripts/build-main.mjs

# 3. pack into a temp dir --------------------------------------------------
WORK="$(mktemp -d)"
# Stop the auto-spawned daemon (its project dir lives under $WORK) before
# deleting the temp tree, otherwise it lingers pointing at a removed binary.
cleanup() {
  if [[ -n "${CG:-}" && -x "${CG:-/nonexistent}" && -n "${FIXTURE:-}" ]]; then
    "$CG" -C "$FIXTURE" server stop >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT
PLAT_TGZ="$(cd "npm/dist/cli-${HOST_OS}-${HOST_CPU}" && npm pack --silent --pack-destination "$WORK")"
MAIN_TGZ="$(cd npm/dist/cli && npm pack --silent --pack-destination "$WORK")"
echo "── packed: $PLAT_TGZ, $MAIN_TGZ"

# 4. install into a throwaway consumer project -----------------------------
CONSUMER="$WORK/consumer"
mkdir -p "$CONSUMER"
( cd "$CONSUMER" && npm init -y >/dev/null 2>&1 \
  && npm install --no-audit --no-fund --silent "$WORK/$PLAT_TGZ" "$WORK/$MAIN_TGZ" )
CG="$CONSUMER/node_modules/.bin/coregraph"
[[ -x "$CG" ]] || { echo "✖ launcher not installed at $CG"; exit 1; }

# Run against a COPY of the fixture inside $WORK so the analysis (and the daemon
# it spawns, which writes .coregraph/ state) never touches the tracked repo tree.
FIXTURE="$WORK/fixture"
cp -R "$REPO_ROOT/tests/e2e/golden/01-ts-single-package" "$FIXTURE"
rm -rf "$FIXTURE/.coregraph"

# 4.1 --version
echo "── 1/3 coregraph --version"
"$CG" --version

# 4.2 real query (verifies the binary analyzes a project)
echo "── 2/3 coregraph query (real analysis)"
"$CG" -C "$FIXTURE" query UserController --output-format json \
  | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const j=JSON.parse(s);if(j.center&&j.center.name==="UserController"){console.log("   ✔ query returned UserController");}else{console.error("   ✖ unexpected query result");process.exit(1);}})'

# 4.3 MCP stdio round-trip
echo "── 3/3 coregraph mcp (stdio JSON-RPC round-trip)"
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize"}' \
  | "$CG" -C "$FIXTURE" mcp \
  | head -1 \
  | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const j=JSON.parse(s);if(j.result&&j.result.serverInfo&&j.result.serverInfo.name==="coregraph"){console.log("   ✔ MCP initialize ->",j.result.serverInfo.name,j.result.serverInfo.version);}else{console.error("   ✖ unexpected MCP response:",s);process.exit(1);}})'

echo "── ✔ local npm packaging verification passed for ${HOST_OS}-${HOST_CPU}"
