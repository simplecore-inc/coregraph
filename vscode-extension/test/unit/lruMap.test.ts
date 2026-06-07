import { strict as assert } from 'node:assert';
import { describe, it } from 'mocha';
import { LRUMap } from '../../src/util/lruMap';

describe('LRUMap', () => {
  it('stores and retrieves entries up to maxSize', () => {
    const m = new LRUMap<string, number>(3);
    m.set('a', 1);
    m.set('b', 2);
    m.set('c', 3);
    assert.equal(m.size, 3);
    assert.equal(m.get('a'), 1);
    assert.equal(m.get('b'), 2);
    assert.equal(m.get('c'), 3);
  });

  it('evicts the oldest insertion when exceeding maxSize', () => {
    const m = new LRUMap<string, number>(2);
    m.set('a', 1);
    m.set('b', 2);
    m.set('c', 3);
    assert.equal(m.size, 2);
    assert.equal(m.has('a'), false, 'oldest entry "a" should be evicted');
    assert.equal(m.get('b'), 2);
    assert.equal(m.get('c'), 3);
  });

  it('get does not change eviction order (FIFO, not LRU)', () => {
    const m = new LRUMap<string, number>(2);
    m.set('a', 1);
    m.set('b', 2);
    // Accessing 'a' does not promote it — insertion order is preserved.
    m.get('a');
    m.set('c', 3);
    assert.equal(m.has('a'), false, '"a" should still be the eviction target');
    assert.equal(m.has('b'), true);
    assert.equal(m.has('c'), true);
  });

  it('overwrites existing key without changing size', () => {
    const m = new LRUMap<string, number>(2);
    m.set('a', 1);
    m.set('b', 2);
    m.set('a', 99);
    assert.equal(m.size, 2);
    assert.equal(m.get('a'), 99);
    assert.equal(m.get('b'), 2);
  });

  it('delete removes the entry and returns true when present', () => {
    const m = new LRUMap<string, number>(3);
    m.set('a', 1);
    assert.equal(m.delete('a'), true);
    assert.equal(m.has('a'), false);
    assert.equal(m.delete('a'), false);
  });

  it('clear empties the map', () => {
    const m = new LRUMap<string, number>(3);
    m.set('a', 1);
    m.set('b', 2);
    m.clear();
    assert.equal(m.size, 0);
    assert.equal(m.has('a'), false);
  });

  it('keys() returns insertion order', () => {
    const m = new LRUMap<string, number>(3);
    m.set('x', 1);
    m.set('y', 2);
    m.set('z', 3);
    assert.deepEqual([...m.keys()], ['x', 'y', 'z']);
  });

  it('throws on non-positive maxSize', () => {
    assert.throws(() => new LRUMap<string, number>(0));
    assert.throws(() => new LRUMap<string, number>(-1));
  });
});
