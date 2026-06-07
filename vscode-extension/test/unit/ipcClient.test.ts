import { strict as assert } from 'node:assert';
import { describe, it, beforeEach, afterEach } from 'mocha';
import * as net from 'node:net';
import * as os from 'node:os';
import * as path from 'node:path';
import * as fs from 'node:fs';
import { IpcClient, CoreGraphError } from '../../src/ipc/client';

const makeSocketPath = () =>
  path.join(os.tmpdir(), `ipc-test-${process.pid}-${Date.now()}.sock`);

describe('IpcClient', function () {
  this.timeout(10000);

  let server: net.Server;
  let sockPath: string;

  beforeEach((done) => {
    sockPath = makeSocketPath();
    try {
      fs.unlinkSync(sockPath);
    } catch {
      /* ok */
    }
    server = net.createServer((conn) => {
      let buf = '';
      conn.on('data', (chunk) => {
        buf += chunk.toString();
        const idx = buf.indexOf('\n');
        if (idx === -1) return;
        const req = JSON.parse(buf.slice(0, idx));
        if (req.method === 'health') {
          const envelope = {
            ok: true,
            body: JSON.stringify({
              ok: true,
              version: 'test-0.0.0',
              cached: true,
              symbols: 42,
            }),
            error: null,
          };
          conn.write(JSON.stringify(envelope) + '\n');
        } else if (req.method === 'error_test') {
          conn.write(
            JSON.stringify({
              ok: false,
              body: '',
              error: 'simulated failure',
            }) + '\n',
          );
        } else {
          conn.write(
            JSON.stringify({
              ok: false,
              body: '',
              error: 'unknown method: ' + req.method,
            }) + '\n',
          );
        }
      });
    });
    server.listen(sockPath, () => done());
  });

  afterEach((done) => {
    server.close(() => {
      try {
        fs.unlinkSync(sockPath);
      } catch {
        /* ok */
      }
      done();
    });
  });

  it('health returns a parsed body with version', async () => {
    const client = new IpcClient(sockPath);
    const h = await client.health();
    assert.equal(h.version, 'test-0.0.0');
    assert.equal(h.symbols, 42);
    assert.equal(h.cached, true);
  });

  it('throws CoreGraphError on ok:false response', async () => {
    const client = new IpcClient(sockPath);
    await assert.rejects(
      () => client.request('error_test', {}),
      (err: Error) => {
        return err instanceof CoreGraphError && err.message.includes('simulated failure');
      },
    );
  });

  it('rejects on connection refused when socket missing', async () => {
    const bogus = path.join(os.tmpdir(), 'coregraph-e2-missing.sock');
    const client = new IpcClient(bogus, 2000);
    await assert.rejects(() => client.health());
  });

  it('calls spawnFn and retries once on ENOENT when retryOnRefuse is set', async () => {
    let spawnCalled = 0;
    const bogus = path.join(
      os.tmpdir(),
      `coregraph-e3-missing-${process.pid}-${Date.now()}.sock`,
    );
    const client = new IpcClient(bogus, 1500, {
      retryOnRefuse: true,
      spawnDelayMs: 50,
      spawnFn: async () => {
        spawnCalled += 1;
        // No actual spawn — we just want to assert spawnFn is invoked.
        // The second attempt also fails (no listener), so the whole
        // request still rejects — but spawnFn was called exactly once.
      },
    });
    await assert.rejects(() => client.health());
    assert.equal(spawnCalled, 1, 'spawnFn should be called exactly once');
  });

  it('does not call spawnFn when retryOnRefuse is false', async () => {
    let spawnCalled = 0;
    const bogus = path.join(
      os.tmpdir(),
      `coregraph-e3-noretry-${process.pid}-${Date.now()}.sock`,
    );
    const client = new IpcClient(bogus, 1000, {
      // retryOnRefuse default false
      spawnFn: async () => { spawnCalled += 1; },
    });
    await assert.rejects(() => client.health());
    assert.equal(spawnCalled, 0);
  });

  it('succeeds after spawn when spawnFn brings up a server', async () => {
    // Start with no server. The spawnFn creates one on demand.
    const serverSock = path.join(
      os.tmpdir(),
      `coregraph-e3-respawn-${process.pid}-${Date.now()}.sock`,
    );
    // Ensure no leftover file.
    try { fs.unlinkSync(serverSock); } catch { /* ok */ }

    let respawnServer: net.Server | null = null;
    const client = new IpcClient(serverSock, 3000, {
      retryOnRefuse: true,
      spawnDelayMs: 100,
      spawnFn: async () => {
        respawnServer = net.createServer((conn) => {
          conn.on('data', () => {
            const envelope = {
              ok: true,
              body: JSON.stringify({
                ok: true,
                version: 'respawned',
                cached: true,
                symbols: 0,
              }),
              error: null,
            };
            conn.write(JSON.stringify(envelope) + '\n');
          });
        });
        await new Promise<void>((resolve) => {
          respawnServer!.listen(serverSock, () => resolve());
        });
      },
    });

    try {
      const h = await client.health();
      assert.equal(h.version, 'respawned');
    } finally {
      if (respawnServer) {
        await new Promise<void>((r) => (respawnServer as net.Server).close(() => r()));
      }
      try { fs.unlinkSync(serverSock); } catch { /* ok */ }
    }
  });

  it('fires onRequestSuccess on successful response', async () => {
    const client = new IpcClient(sockPath);
    const successes: string[] = [];
    const failures: string[] = [];
    client.onRequestSuccess((e) => successes.push(e.method));
    client.onRequestFailed((e) => failures.push(e.method));

    await client.health();
    assert.deepEqual(successes, ['health']);
    assert.deepEqual(failures, []);
  });

  it('fires onRequestFailed on ok:false response', async () => {
    const client = new IpcClient(sockPath);
    const successes: string[] = [];
    const failures: { method: string; msg: string }[] = [];
    client.onRequestSuccess((e) => successes.push(e.method));
    client.onRequestFailed((e) => failures.push({ method: e.method, msg: e.error.message }));

    await assert.rejects(() => client.request('error_test', {}));
    assert.deepEqual(successes, []);
    assert.equal(failures.length, 1);
    assert.equal(failures[0].method, 'error_test');
    assert.ok(failures[0].msg.includes('simulated failure'));
  });

  it('fires onRequestFailed on connection refused (no retry)', async () => {
    const bogus = path.join(os.tmpdir(), `coregraph-noconn-${process.pid}-${Date.now()}.sock`);
    const client = new IpcClient(bogus, 1000);
    const failures: string[] = [];
    client.onRequestFailed((e) => failures.push(e.method));

    await assert.rejects(() => client.health());
    assert.deepEqual(failures, ['health']);
  });

  it('fires failed-then-success on retry path', async () => {
    // First attempt hits a missing socket, spawnFn brings up a server, retry succeeds.
    const serverSock = path.join(
      os.tmpdir(),
      `coregraph-retry-evt-${process.pid}-${Date.now()}.sock`,
    );
    try { fs.unlinkSync(serverSock); } catch { /* ok */ }

    let respawnServer: net.Server | null = null;
    const client = new IpcClient(serverSock, 3000, {
      retryOnRefuse: true,
      spawnDelayMs: 30,
      spawnFn: async () => {
        respawnServer = net.createServer((conn) => {
          conn.on('data', () => {
            conn.write(
              JSON.stringify({
                ok: true,
                body: JSON.stringify({
                  ok: true,
                  version: 'retry-ok',
                  cached: true,
                  symbols: 0,
                }),
                error: null,
              }) + '\n',
            );
          });
        });
        await new Promise<void>((r) => respawnServer!.listen(serverSock, () => r()));
      },
    });
    const events: string[] = [];
    client.onRequestSuccess(() => events.push('success'));
    client.onRequestFailed(() => events.push('failed'));

    try {
      await client.health();
      assert.deepEqual(events, ['failed', 'success']);
    } finally {
      if (respawnServer) await new Promise<void>((r) => (respawnServer as net.Server).close(() => r()));
      try { fs.unlinkSync(serverSock); } catch { /* ok */ }
    }
  });

  it('timeoutMs parameter overrides the constructor default', async () => {
    // Use a server that accepts but never replies so we hit the timeout
    // path. Track connections so we can destroy them on teardown —
    // `server.close()` otherwise waits for clients to hang up.
    const silentSock = path.join(
      os.tmpdir(),
      `coregraph-silent-${process.pid}-${Date.now()}.sock`,
    );
    try { fs.unlinkSync(silentSock); } catch { /* ok */ }
    const conns: net.Socket[] = [];
    const silent = net.createServer((conn) => {
      conns.push(conn);
    });
    await new Promise<void>((r) => silent.listen(silentSock, () => r()));

    try {
      // Constructor default 30 000ms; override at call site with 80ms
      // so the request rejects quickly without blocking the suite.
      const client = new IpcClient(silentSock, 30000);
      const started = Date.now();
      await assert.rejects(
        () => client.health(80),
        (err: Error) => err instanceof CoreGraphError && err.message.includes('80ms'),
      );
      const elapsed = Date.now() - started;
      // Loose bound — should be well under 500ms on any CI machine.
      assert.ok(elapsed < 500, `expected quick timeout, got ${elapsed}ms`);
    } finally {
      for (const c of conns) c.destroy();
      await new Promise<void>((r) => silent.close(() => r()));
      try { fs.unlinkSync(silentSock); } catch { /* ok */ }
    }
  });

  it('dispose clears event listeners', async () => {
    const client = new IpcClient(sockPath);
    const events: string[] = [];
    client.onRequestSuccess(() => events.push('success'));

    client.dispose();
    await client.health();
    assert.deepEqual(events, []);
  });

  it('event listener disposable unsubscribes', async () => {
    const client = new IpcClient(sockPath);
    const events: string[] = [];
    const sub = client.onRequestSuccess(() => events.push('success'));

    await client.health();
    sub.dispose();
    await client.health();
    assert.deepEqual(events, ['success']);
  });

  it('dedupes concurrent spawnFn calls across parallel requests', async () => {
    const bogus = path.join(
      os.tmpdir(),
      `coregraph-dedup-${process.pid}-${Date.now()}.sock`,
    );
    try { fs.unlinkSync(bogus); } catch { /* ok */ }

    let spawnCalls = 0;
    const client = new IpcClient(bogus, 500, {
      retryOnRefuse: true,
      spawnDelayMs: 50,
      spawnFn: async () => {
        spawnCalls += 1;
        // Simulate slow spawn so concurrent callers pile up while the
        // first spawn is in flight. Without dedup, each caller would
        // trigger its own spawnFn invocation.
        await new Promise((r) => setTimeout(r, 40));
      },
    });

    // Fire 5 requests concurrently. All should ENOENT, all should see
    // the same in-flight spawn, so spawnFn must run exactly once.
    await Promise.allSettled([
      client.health(),
      client.health(),
      client.health(),
      client.health(),
      client.health(),
    ]);

    assert.equal(
      spawnCalls,
      1,
      `spawnFn should run exactly once for concurrent requests, got ${spawnCalls}`,
    );
  });
});
