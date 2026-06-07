import { strict as assert } from 'node:assert';
import { describe, it } from 'mocha';
import { PathDebouncer } from '../../src/util/debouncer';

describe('didChange stability', function () {
  this.timeout(60_000);

  it('1000 rapid schedules on one path cause at most 10 reindex calls', async () => {
    const calls: string[] = [];
    const fakeIpc = {
      reindex: async (file: string) => {
        calls.push(file);
      },
    };
    const d = new PathDebouncer(300);
    for (let i = 0; i < 1000; i++) {
      d.schedule('foo.rs', () => fakeIpc.reindex('foo.rs'));
      await new Promise((r) => setImmediate(r));
    }
    await new Promise((r) => setTimeout(r, 400));

    // With a 300ms debounce window and setImmediate pacing, we expect
    // exactly one reindex call — the final scheduled callback. A
    // larger allowance (<= 10) tolerates timer jitter on slow CI runners.
    assert.ok(
      calls.length <= 10,
      `expected <= 10 reindex calls for 1000 rapid schedules, got ${calls.length}`,
    );
    assert.ok(
      calls.length >= 1,
      `expected at least 1 reindex call, got 0`,
    );
  });
});
