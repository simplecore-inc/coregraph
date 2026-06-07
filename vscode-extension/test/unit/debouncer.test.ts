import { strict as assert } from 'node:assert';
import { describe, it } from 'mocha';
import { PathDebouncer } from '../../src/util/debouncer';

describe('PathDebouncer', () => {
  it('fires once after the delay', async () => {
    const d = new PathDebouncer(50);
    let n = 0;
    d.schedule('a', () => { n++; });
    await sleep(100);
    assert.equal(n, 1);
  });

  it('reschedules reset the timer for the same path', async () => {
    const d = new PathDebouncer(50);
    let n = 0;
    d.schedule('a', () => { n++; });
    await sleep(30);
    // Second schedule BEFORE the first fires — should cancel the first.
    d.schedule('a', () => { n++; });
    await sleep(30);
    assert.equal(n, 0, 'first callback should have been cancelled by reschedule');
    await sleep(50);
    assert.equal(n, 1);
  });

  it('different paths debounce independently', async () => {
    const d = new PathDebouncer(50);
    let a = 0;
    let b = 0;
    d.schedule('a', () => { a++; });
    d.schedule('b', () => { b++; });
    await sleep(100);
    assert.equal(a, 1);
    assert.equal(b, 1);
  });

  it('cancelAll prevents pending callbacks from firing', async () => {
    const d = new PathDebouncer(50);
    let n = 0;
    d.schedule('a', () => { n++; });
    d.schedule('b', () => { n++; });
    d.cancelAll();
    await sleep(100);
    assert.equal(n, 0);
  });

  it('accepts async callbacks and does not swallow them', async () => {
    const d = new PathDebouncer(20);
    let resolved = false;
    d.schedule('a', async () => {
      await sleep(5);
      resolved = true;
    });
    await sleep(60);
    assert.equal(resolved, true);
  });
});

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
