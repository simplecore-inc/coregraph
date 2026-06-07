import { strict as assert } from 'node:assert';
import { describe, it, beforeEach, afterEach } from 'mocha';
import { resolveSocketPath } from '../../src/ipc/paths';

describe('resolveSocketPath', () => {
  const origXdg = process.env.XDG_RUNTIME_DIR;
  const origUsername = process.env.USERNAME;

  beforeEach(() => {
    delete process.env.XDG_RUNTIME_DIR;
    delete process.env.USERNAME;
  });

  afterEach(() => {
    if (origXdg !== undefined) process.env.XDG_RUNTIME_DIR = origXdg;
    else delete process.env.XDG_RUNTIME_DIR;
    if (origUsername !== undefined) process.env.USERNAME = origUsername;
    else delete process.env.USERNAME;
  });

  it('uses XDG_RUNTIME_DIR when set on Unix', () => {
    process.env.XDG_RUNTIME_DIR = '/tmp/run-test';
    const p = resolveSocketPath({ platform: 'linux' });
    assert.equal(p, '/tmp/run-test/coregraph/server.sock');
  });

  it('falls back to explicit home on Unix without XDG', () => {
    const p = resolveSocketPath({ platform: 'linux', home: '/Users/test' });
    assert.equal(p, '/Users/test/.coregraph/server.sock');
  });

  it('uses os.homedir() on Unix when no home override and no XDG', () => {
    const p = resolveSocketPath({ platform: 'darwin' });
    // Don't assert a specific home — just confirm the function returned
    // a path ending in the expected suffix.
    assert.ok(p.endsWith('/.coregraph/server.sock'), `expected suffix match, got ${p}`);
  });

  it('returns named pipe on Windows with explicit username', () => {
    const p = resolveSocketPath({ platform: 'win32', username: 'alice' });
    assert.equal(p, '\\\\.\\pipe\\coregraph-alice');
  });

  it('uses USERNAME env on Windows when no username override', () => {
    process.env.USERNAME = 'bob';
    const p = resolveSocketPath({ platform: 'win32' });
    assert.equal(p, '\\\\.\\pipe\\coregraph-bob');
  });

  it('defaults Windows username to "default" when env unset and no override', () => {
    const p = resolveSocketPath({ platform: 'win32' });
    assert.equal(p, '\\\\.\\pipe\\coregraph-default');
  });
});
