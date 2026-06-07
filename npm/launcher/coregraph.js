#!/usr/bin/env node
'use strict';

// Launcher for the `coregraph` command. This file ships in the main package
// (@coregraph/cli). The real native binary lives in a per-platform optional
// dependency named `<main>-<os>-<arch>` (e.g. @coregraph/cli-darwin-arm64);
// npm installs only the one matching the host's os/cpu. We resolve that
// package's binary and exec it, forwarding argv, stdio, exit code, and signals.

const { spawnSync } = require('node:child_process');
const { join } = require('node:path');

// Derive the platform package name from our own package.json so the package
// identity has a single source of truth (no duplicated scope string here).
function mainPackageName() {
  try {
    return require(join(__dirname, '..', 'package.json')).name;
  } catch {
    return '@coregraph/cli';
  }
}

function resolveBinary() {
  const { platform, arch } = process;
  const pkg = `${mainPackageName()}-${platform}-${arch}`;
  const binName = platform === 'win32' ? 'coregraph.exe' : 'coregraph';
  try {
    return require.resolve(`${pkg}/bin/${binName}`);
  } catch {
    return null;
  }
}

const binary = resolveBinary();
if (!binary) {
  const { platform, arch } = process;
  process.stderr.write(
    `coregraph: no prebuilt binary for ${platform}-${arch}.\n` +
      'Supported platforms: darwin-arm64, darwin-x64, linux-x64, linux-arm64, win32-x64.\n' +
      'If your platform is listed, reinstall with optional dependencies enabled:\n' +
      `  npm install -g ${mainPackageName()} --include=optional\n`
  );
  process.exit(1);
}

// Forward everything to the native binary. It self-spawns its background daemon
// via std::env::current_exe(), which resolves to this real binary (not node),
// so the daemon runs without node in the loop.
const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  process.stderr.write(`coregraph: failed to launch ${binary}: ${result.error.message}\n`);
  process.exit(1);
}
if (result.signal) {
  // Re-raise the terminating signal so the parent shell sees the true cause.
  process.kill(process.pid, result.signal);
}
process.exit(result.status === null ? 1 : result.status);
