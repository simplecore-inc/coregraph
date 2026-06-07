/**
 * Bounded FIFO map. Evicts the oldest inserted entry when `size`
 * exceeds `maxSize`. `get` does NOT promote keys (access order is not
 * tracked — this is strict insertion-order FIFO, not true LRU).
 *
 * Use case: bound unbounded provider-side caches (per-file symbol
 * lookups, CodeLens entries) so long-running VSCode sessions do not
 * grow without limit.
 */
export class LRUMap<K, V> {
  private readonly data = new Map<K, V>();

  constructor(private readonly maxSize: number) {
    if (maxSize <= 0) {
      throw new Error(`LRUMap: maxSize must be positive, got ${maxSize}`);
    }
  }

  get(key: K): V | undefined {
    return this.data.get(key);
  }

  set(key: K, value: V): void {
    if (this.data.has(key)) {
      this.data.set(key, value);
      return;
    }
    this.data.set(key, value);
    if (this.data.size > this.maxSize) {
      const oldest = this.data.keys().next().value as K;
      this.data.delete(oldest);
    }
  }

  has(key: K): boolean {
    return this.data.has(key);
  }

  delete(key: K): boolean {
    return this.data.delete(key);
  }

  clear(): void {
    this.data.clear();
  }

  get size(): number {
    return this.data.size;
  }

  keys(): IterableIterator<K> {
    return this.data.keys();
  }
}
