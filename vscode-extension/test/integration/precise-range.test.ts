import { strict as assert } from 'node:assert';
import { describe, it, before, after } from 'mocha';
import { spawn, ChildProcess } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as os from 'node:os';
import { IpcClient } from '../../src/ipc/client';
import { resolveSocketPath } from '../../src/ipc/paths';

describe('precise range (E2E)', function () {
  this.timeout(30_000);

  const tempProj = fs.mkdtempSync(path.join(os.tmpdir(), 'cg-e2e-'));
  let daemon: ChildProcess | undefined;

  before(async () => {
    // Tiny Rust crate with a single symbol at a known line.
    fs.writeFileSync(
      path.join(tempProj, 'Cargo.toml'),
      [
        '[package]',
        'name = "cg_e2e"',
        'version = "0.0.0"',
        'edition = "2021"',
        '',
        '[lib]',
        'path = "src/lib.rs"',
        '',
      ].join('\n'),
    );
    fs.mkdirSync(path.join(tempProj, 'src'));
    // Line 0: // prelude
    // Line 1: // padding
    // Line 2: pub fn target() {} <- asserted below
    fs.writeFileSync(
      path.join(tempProj, 'src', 'lib.rs'),
      '// prelude\n// padding\npub fn target() {}\n',
    );

    // Locate the binary. Prefer release; fall back to debug. If
    // neither exists, the test harness skips by throwing in `before`
    // so mocha reports the suite as failed — devs running integration
    // tests must have a built binary.
    const workspace = path.resolve(__dirname, '../../..');
    const candidates = [
      path.join(workspace, 'target', 'release', 'coregraph'),
      path.join(workspace, 'target', 'debug', 'coregraph'),
    ];
    const binary = candidates.find((p) => fs.existsSync(p));
    if (!binary) {
      throw new Error(
        `coregraph binary not found; build with ` +
          `\`cargo build -p coregraph --release\` then rerun this test. ` +
          `Searched: ${candidates.join(', ')}`,
      );
    }

    // Stop any lingering daemon and remove a stale socket file.
    // A stale socket causes EADDRINUSE on daemon start and ECONNREFUSED
    // on health polls (full 1500ms per attempt), exhausting the 30s budget.
    try {
      spawn(binary, ['server', 'stop'], { stdio: 'ignore' });
      await new Promise((r) => setTimeout(r, 300));
    } catch { /* ignore */ }
    try { fs.unlinkSync(resolveSocketPath()); } catch { /* ok if absent */ }

    daemon = spawn(
      binary,
      ['-C', tempProj, 'server', 'start', '--foreground'],
      { detached: true, stdio: 'ignore' },
    );
    daemon.unref();

    // Poll the socket up to ~8s. Pass tempProj so the daemon serves from
    // its initialized graph rather than indexing the test runner's CWD.
    const client = new IpcClient(resolveSocketPath(), 500);
    for (let i = 0; i < 40; i++) {
      try {
        await client.request('health', {}, tempProj);
        return;
      } catch { /* keep polling */ }
      await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error('daemon did not become healthy within 8s');
  });

  after(async () => {
    if (daemon && !daemon.killed) {
      try { daemon.kill('SIGTERM'); } catch { /* ignore */ }
    }
    // Best-effort cleanup. On Unix the socket is a real fs entry;
    // leave the temp project in place for post-mortem if the test failed.
    try {
      fs.rmSync(tempProj, { recursive: true, force: true });
    } catch { /* ignore */ }
  });

  it('lsp.definition for "target" returns line 2 (0-based)', async () => {
    const client = new IpcClient(resolveSocketPath(), 5000);
    const body: any = await client.request(
      'lsp.definition',
      { symbol: 'target' },
      tempProj,
    );
    assert.ok(
      Array.isArray(body.locations) && body.locations.length > 0,
      `expected at least one location, got ${JSON.stringify(body)}`,
    );
    const range = body.locations[0].range;
    assert.ok(range, `expected range field in location, got ${JSON.stringify(body.locations[0])}`);
    assert.equal(
      range.start.line,
      2,
      `expected start.line == 2 (0-based, after two comment lines), got ${range.start.line}`,
    );
  });
});
