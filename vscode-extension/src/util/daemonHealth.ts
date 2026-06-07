/**
 * Daemon liveness state machine shared across providers.
 *
 * - `offline`: initial state, and after a request fails with a
 *   connection-level error.
 * - `starting`: a spawn-and-retry attempt is in flight.
 * - `online`: last observed request succeeded.
 *
 * The tracker is purely observational — providers subscribe via
 * `onDidChangeState` to refresh UI (status bar badge, tree view nudges,
 * CodeLens invalidation). Transitions are driven by the IPC client
 * (success → online, retry begin → starting, failure → offline).
 */
export type DaemonState = "starting" | "online" | "offline";

type Listener<T> = (e: T) => void;

export interface Disposable {
  dispose(): void;
}

export type EventFn<T> = (listener: Listener<T>) => Disposable;

export interface DaemonHealthTracker {
  readonly state: DaemonState;
  readonly onDidChangeState: EventFn<DaemonState>;
  setState(next: DaemonState): void;
  dispose(): void;
}

export function createDaemonHealthTracker(): DaemonHealthTracker {
  let current: DaemonState = "offline";
  const listeners: Listener<DaemonState>[] = [];

  const onDidChangeState: EventFn<DaemonState> = (listener) => {
    listeners.push(listener);
    return {
      dispose: () => {
        const i = listeners.indexOf(listener);
        if (i >= 0) listeners.splice(i, 1);
      },
    };
  };

  return {
    get state(): DaemonState {
      return current;
    },
    onDidChangeState,
    setState(next: DaemonState): void {
      if (next === current) return;
      current = next;
      // Snapshot listeners so a listener that unsubscribes during dispatch
      // does not skip sibling listeners (splice would shift indices mid-loop).
      const snapshot = listeners.slice();
      for (const l of snapshot) l(next);
    },
    dispose(): void {
      listeners.length = 0;
    },
  };
}
