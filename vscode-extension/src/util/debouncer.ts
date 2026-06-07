/**
 * Per-path debouncer. Each call to `schedule(path, fn)` cancels any
 * previously scheduled callback for the same `path` and queues `fn`
 * to run after `delayMs`. Different paths are independent.
 *
 * Designed for coalescing rapid VSCode `onDidChangeTextDocument`
 * events so the daemon is not hammered with reindex calls during
 * keystroke bursts.
 */
export class PathDebouncer {
  private readonly timers = new Map<string, NodeJS.Timeout>();

  constructor(private readonly delayMs: number) {}

  /**
   * Schedule `fn` to run for `path` after `delayMs`. If there is
   * already a pending timer for the same path, it is cancelled and
   * replaced. The callback can be sync or async — async errors are
   * unhandled (intentional: callers wire their own error handling
   * inside the callback body).
   */
  schedule(path: string, fn: () => void | Promise<void>): void {
    const existing = this.timers.get(path);
    if (existing) clearTimeout(existing);
    const t = setTimeout(() => {
      this.timers.delete(path);
      void fn();
    }, this.delayMs);
    this.timers.set(path, t);
  }

  /**
   * Cancel every pending timer. Useful on extension deactivate so
   * pending callbacks don't fire after resource cleanup.
   */
  cancelAll(): void {
    for (const t of this.timers.values()) clearTimeout(t);
    this.timers.clear();
  }
}
