import * as os from 'node:os';
import * as path from 'node:path';

/** Options that override environment detection. Useful for tests and
 * for callers that know the target platform more precisely than the
 * current `process.platform`. */
export interface ResolveOptions {
  /** Override `process.platform`. */
  platform?: NodeJS.Platform;
  /** Override the home directory used for the Unix fallback. */
  home?: string;
  /** Override the Windows username. Ignored on Unix. */
  username?: string;
}

/** Return the IPC endpoint string for the running platform.
 *
 * Mirrors `coregraph_cli::ipc::socket_path`:
 * - On Windows: `\\.\pipe\coregraph-<USERNAME>` (named pipe, not a fs path).
 * - On Unix with `XDG_RUNTIME_DIR` set: `$XDG_RUNTIME_DIR/coregraph/server.sock`.
 * - On Unix without XDG: `<home>/.coregraph/server.sock`.
 *
 * The Rust daemon binds to the same endpoint. Both sides must agree on
 * the format exactly or the IPC client cannot reach the server.
 */
export function resolveSocketPath(opts: ResolveOptions = {}): string {
  const platform = opts.platform ?? process.platform;
  if (platform === 'win32') {
    const user = opts.username ?? process.env.USERNAME ?? 'default';
    return `\\\\.\\pipe\\coregraph-${user}`;
  }
  const xdg = process.env.XDG_RUNTIME_DIR;
  if (xdg) {
    // Force POSIX separators on the Unix branch so tests that pass an
    // explicit `platform: 'linux'|'darwin'` produce `/`-separated output
    // even when run from a Windows CI host. On real Unix runtime,
    // `path.posix.join` and `path.join` are identical; the Windows
    // runtime never reaches this branch because the `win32` guard above
    // short-circuits.
    return path.posix.join(xdg, 'coregraph', 'server.sock');
  }
  const home = opts.home ?? os.homedir();
  return path.posix.join(home, '.coregraph', 'server.sock');
}
