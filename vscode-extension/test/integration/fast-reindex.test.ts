import { strict as assert } from 'node:assert';
import { describe, it, before, after } from 'mocha';
import { spawn, ChildProcess } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as os from 'node:os';
import { IpcClient } from '../../src/ipc/client';
import { resolveSocketPath } from '../../src/ipc/paths';

describe('fast reindex (E2E)', function () {
  this.timeout(60_000);

  const tempProj = fs.mkdtempSync(path.join(os.tmpdir(), 'cg-fastreindex-'));
  const libPath = path.join(tempProj, 'src', 'lib.rs');
  let daemon: ChildProcess | undefined;

  before(async () => {
    // Minimal Rust crate with two functions. We use a single-file project
    // so the extracted symbols are predictable; cross-file relationships
    // are intentionally deferred to a later test once name-based re-linking
    // lands in task 0.5c.
    fs.writeFileSync(
      path.join(tempProj, 'Cargo.toml'),
      [
        '[package]',
        'name = "cg_fr_e2e"',
        'version = "0.0.0"',
        'edition = "2021"',
        '',
        '[lib]',
        'path = "src/lib.rs"',
        '',
      ].join('\n'),
    );
    fs.mkdirSync(path.join(tempProj, 'src'));
    fs.writeFileSync(libPath, 'pub fn alpha() {}\npub fn beta() {}\n');

    // Find the binary: prefer debug (most recent build). Release is also
    // accepted if it is present and debug is absent.
    const workspace = path.resolve(__dirname, '../../..');
    const candidates = [
      path.join(workspace, 'target', 'debug', 'coregraph'),
      path.join(workspace, 'target', 'release', 'coregraph'),
    ];
    const binary = candidates.find((p) => fs.existsSync(p));
    if (!binary) {
      throw new Error(
        `coregraph binary not found; build with ` +
          `\`cargo build -p coregraph\` then rerun this test. ` +
          `Searched: ${candidates.join(', ')}`,
      );
    }

    // Stop any lingering daemon and remove a stale socket file.
    // The socket path persists on disk after a daemon crash, causing
    // EADDRINUSE on the next start and ECONNREFUSED on health polls
    // (which use the full 1500ms timeout each, exhausting the 30s budget).
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

    // Poll up to ~8s for readiness. Pass `tempProj` as the project so
    // the daemon serves from its initialized graph rather than trying to
    // load the test runner's CWD (which would trigger a slow index pass).
    // Use a short per-probe timeout: a healthy daemon responds in <50ms.
    const probe = new IpcClient(resolveSocketPath(), 500);
    for (let i = 0; i < 40; i++) {
      try {
        await probe.request('health', {}, tempProj);
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
    try {
      fs.rmSync(tempProj, { recursive: true, force: true });
    } catch { /* ignore */ }
  });

  it('fast reindex returns honest telemetry', async () => {
    const client = new IpcClient(resolveSocketPath(), 10_000);
    const result = await client.reindex(libPath, 'fast', tempProj);

    assert.equal(result.mode, 'fast');
    assert.equal(result.reindexed, true);
    assert.equal(result.file_missing, false);
    assert.equal(typeof result.elapsed_ms, 'number');
    assert.equal(typeof result.nodes_removed, 'number');
    assert.equal(typeof result.nodes_inserted, 'number');
    assert.equal(typeof result.edges_removed_with_evidence, 'number');
    // Single-file project has no cross-file edges, so both re-link counters are 0.
    assert.equal(result.cross_file_edges_staled, 0);
    assert.equal(result.cross_file_edges_dropped, 0);
  });

  it('fast reindex reflects file edits via lsp.workspace_symbol', async () => {
    const client = new IpcClient(resolveSocketPath(), 10_000);

    // Baseline: "alpha" should be present in the initial graph built during
    // daemon startup.
    const initial: any = await client.request(
      'lsp.workspace_symbol',
      { query: 'alpha' },
      tempProj,
    );
    assert.ok(
      Array.isArray(initial.symbols) && initial.symbols.some((s: any) => s.name === 'alpha'),
      `expected "alpha" in initial symbols, got ${JSON.stringify(initial)}`,
    );

    // Edit the file on disk: rename "alpha" → "renamed_alpha".
    fs.writeFileSync(libPath, 'pub fn renamed_alpha() {}\npub fn beta() {}\n');

    // Fast reindex — must target tempProj so the daemon updates the correct
    // graph rather than the test runner's CWD.
    const reindexResult = await client.reindex(libPath, 'fast', tempProj);
    assert.equal(reindexResult.mode, 'fast');
    assert.equal(reindexResult.reindexed, true);
    // The daemon uses get_or_load_without_refresh for reindex requests, so
    // the cached graph is returned even if source files have newer mtimes.
    // The surgical reindex then sees the old "alpha" node and removes it,
    // producing nodes_removed >= 1.
    assert.ok(
      (reindexResult.nodes_removed ?? 0) >= 1,
      `expected nodes_removed >= 1 after rename, got ${reindexResult.nodes_removed}`,
    );

    // Query again: "renamed_alpha" must now appear.
    const after: any = await client.request(
      'lsp.workspace_symbol',
      { query: 'renamed_alpha' },
      tempProj,
    );
    assert.ok(
      Array.isArray(after.symbols) && after.symbols.some((s: any) => s.name === 'renamed_alpha'),
      `expected "renamed_alpha" in post-reindex symbols, got ${JSON.stringify(after)}`,
    );

    // "alpha" (exact name) must be gone. Note: a substring search for
    // "alpha" would also match "renamed_alpha", so we check the exact name.
    const gone: any = await client.request(
      'lsp.workspace_symbol',
      { query: 'alpha' },
      tempProj,
    );
    assert.ok(
      !gone.symbols.some((s: any) => s.name === 'alpha'),
      `expected no symbol named exactly "alpha" after reindex, got ${JSON.stringify(gone)}`,
    );
  });

  it('fast reindex on missing file reports file_missing: true', async () => {
    const client = new IpcClient(resolveSocketPath(), 10_000);
    const ghost = path.join(tempProj, 'src', 'ghost.rs');
    const result = await client.reindex(ghost, 'fast', tempProj);

    assert.equal(result.reindexed, true);
    assert.equal(result.file_missing, true);
    assert.equal(result.mode, 'fast');
    // A missing file has no nodes to re-extract, so nodes_inserted is 0.
    assert.equal(result.nodes_inserted, 0);
  });

  it('fast reindex response includes cross_file_edges_dropped field (0.5c shape)', async () => {
    // Verifies that the new cross_file_edges_dropped field is present in the
    // response shape after the 0.5c re-link implementation. A single-file
    // project has no cross-file edges, so both counters are 0 — but both
    // must be present as numeric fields.
    const client = new IpcClient(resolveSocketPath(), 10_000);
    const result = await client.reindex(libPath, 'fast', tempProj);

    assert.equal(result.mode, 'fast');
    assert.equal(typeof result.cross_file_edges_staled, 'number',
      'cross_file_edges_staled must be a number');
    assert.equal(typeof result.cross_file_edges_dropped, 'number',
      'cross_file_edges_dropped must be a number');
    // Single-file project: no cross-file edges to re-link or drop.
    assert.equal(result.cross_file_edges_staled, 0);
    assert.equal(result.cross_file_edges_dropped, 0);
  });
});
