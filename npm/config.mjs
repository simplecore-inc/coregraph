// Single source of truth for the npm package identity and the platform matrix.
// Every build/publish script and the launcher read from here, so switching the
// published name (scoped <-> unscoped) or adding a platform is a one-file edit.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = join(HERE, '..');

// Published identity. The `coregraph` org on npm owns the `@coregraph` scope,
// so the CLI ships as `@coregraph/cli` with per-platform `@coregraph/cli-*`
// optional dependencies. The user-facing command stays `coregraph` (see BIN).
export const SCOPE = '@coregraph';
export const MAIN_PACKAGE = `${SCOPE}/cli`;
export const BIN = 'coregraph';
export const LICENSE = 'MIT';
export const REPOSITORY = 'https://github.com/simplecore-inc/coregraph';
export const HOMEPAGE = 'https://github.com/simplecore-inc/coregraph#readme';

// os/cpu use Node's process.platform / process.arch vocabulary so the launcher
// can resolve `${MAIN_PACKAGE}-${process.platform}-${process.arch}` directly.
// Linux targets are musl so the binary is statically linked and runs on any
// distro (glibc or musl) without a libc version dependency.
export const PLATFORMS = [
  { os: 'darwin', cpu: 'arm64', rustTarget: 'aarch64-apple-darwin',       binary: 'coregraph' },
  { os: 'darwin', cpu: 'x64',   rustTarget: 'x86_64-apple-darwin',        binary: 'coregraph' },
  { os: 'linux',  cpu: 'x64',   rustTarget: 'x86_64-unknown-linux-musl',  binary: 'coregraph' },
  { os: 'linux',  cpu: 'arm64', rustTarget: 'aarch64-unknown-linux-musl', binary: 'coregraph' },
  { os: 'win32',  cpu: 'x64',   rustTarget: 'x86_64-pc-windows-msvc',     binary: 'coregraph.exe' },
  { os: 'win32',  cpu: 'arm64', rustTarget: 'aarch64-pc-windows-msvc',    binary: 'coregraph.exe' },
];

export function platformPackageName({ os, cpu }) {
  return `${MAIN_PACKAGE}-${os}-${cpu}`;
}

export function platformByKey(os, cpu) {
  return PLATFORMS.find((p) => p.os === os && p.cpu === cpu);
}

// The npm version is the cli crate's version — exactly what `coregraph
// --version` prints — so the JS packages and the shipped binary never diverge.
export function cliVersion() {
  // Single source of truth: the CLI crate inherits the workspace version
  // (`version.workspace = true`), so read the root Cargo.toml [workspace.package] version.
  const cargo = readFileSync(join(REPO_ROOT, 'Cargo.toml'), 'utf8');
  // First line-start `version = "..."` is the [workspace.package] version.
  const m = cargo.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!m) {
    throw new Error('could not read [workspace.package] version from Cargo.toml');
  }
  return m[1];
}
