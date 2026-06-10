import * as net from 'node:net';
import type {
  Response,
  HealthResponse,
  ReindexResult,
  InspectResult,
} from './types';

/** Thrown when the daemon returns `{ok: false, error: "..."}`. */
export class CoreGraphError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CoreGraphError';
  }
}

/** Default per-request timeout. Set at 30 s to cover the first call on
 * a cold daemon which must build the project graph (workspace scans
 * for ~200 files take ~1–2 s on their own; `diff` then runs
 * per-file impact which compounds). Subsequent calls against a warm
 * graph return in a few hundred ms. Callers that know they're
 * always-fast (health, inspect) can pass a shorter value explicitly. */
const DEFAULT_TIMEOUT_MS = 30000;

/** Options controlling auto-spawn behaviour on connection failure. */
export interface IpcOptions {
  /**
   * Called once when a connection attempt fails with ENOENT or
   * ECONNREFUSED. Typical implementation spawns the daemon via
   * `child_process.spawn(binary, ['server', 'start', '--foreground'])`
   * with `stdio: 'ignore'` and `detached: true` + `proc.unref()`.
   *
   * The returned Promise should resolve once the spawn call has
   * returned (not when the daemon is ready — we retry with a small
   * delay after this resolves).
   */
  spawnFn?: () => Promise<void>;

  /**
   * When true, ENOENT/ECONNREFUSED triggers `spawnFn()` + one retry.
   * Default false.
   */
  retryOnRefuse?: boolean;

  /** Milliseconds to wait between spawn and retry. Default 500. */
  spawnDelayMs?: number;
}

/** Resolves after `ms` milliseconds. */
function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

type Listener<T> = (e: T) => void;

/** Structurally compatible with `vscode.Event<T>` so consumers can use
 * `client.onRequestSuccess(handler)` both here (tests, node) and inside
 * VSCode. Returning a `Disposable` matches the VSCode Event contract. */
export interface IpcEventDisposable {
  dispose(): void;
}

export type IpcEvent<T> = (listener: Listener<T>) => IpcEventDisposable;

export interface RequestSuccessEvent {
  method: string;
}

export interface RequestFailedEvent {
  method: string;
  error: Error;
}

/** Stateless IPC client — one `net.createConnection` per request, no
 * persistent socket. There is nothing to dispose; the instance can be
 * freely dropped or retained. Methods are fully re-entrant: multiple
 * `request()` calls in flight use independent sockets.
 *
 * This design matches the daemon's per-request lifecycle and avoids
 * socket-reuse bugs that would otherwise require refcounting. The
 * cost is a ~1 ms connect/teardown overhead per call, which is below
 * the 200 ms CodeLens budget and the 150 ms Hover budget. */
export class IpcClient {
  private readonly successListeners: Listener<RequestSuccessEvent>[] = [];
  private readonly failedListeners: Listener<RequestFailedEvent>[] = [];
  /** Guards concurrent spawn attempts. When multiple providers hit an
   * ENOENT/ECONNREFUSED simultaneously (common on extension activation
   * before the daemon is up), only the first caller runs spawnFn; the
   * rest await the same Promise. Prevents N stray `server start`
   * processes that tie up CPU and the socket. */
  private spawnInFlight: Promise<void> | null = null;

  constructor(
    private readonly socketPath: string,
    private readonly timeoutMs: number = DEFAULT_TIMEOUT_MS,
    private readonly opts: IpcOptions = {},
  ) {}

  /** Fires after every successful daemon response (including retry
   * successes). Consumers wire this to `DaemonHealthTracker.setState("online")`. */
  public readonly onRequestSuccess: IpcEvent<RequestSuccessEvent> = (
    listener,
  ) => {
    this.successListeners.push(listener);
    return {
      dispose: () => {
        const i = this.successListeners.indexOf(listener);
        if (i >= 0) this.successListeners.splice(i, 1);
      },
    };
  };

  /** Fires on any request rejection — connection refused, timeout,
   * daemon-level `{ok: false}`, or retry exhaustion. */
  public readonly onRequestFailed: IpcEvent<RequestFailedEvent> = (
    listener,
  ) => {
    this.failedListeners.push(listener);
    return {
      dispose: () => {
        const i = this.failedListeners.indexOf(listener);
        if (i >= 0) this.failedListeners.splice(i, 1);
      },
    };
  };

  private fireSuccess(method: string): void {
    const snapshot = this.successListeners.slice();
    for (const l of snapshot) l({ method });
  }

  private fireFailed(method: string, error: Error): void {
    const snapshot = this.failedListeners.slice();
    for (const l of snapshot) l({ method, error });
  }

  /** Returns true when the error code warrants a spawn-and-retry attempt. */
  private shouldRetry(e: unknown): boolean {
    if (!this.opts.retryOnRefuse) return false;
    const code = (e as NodeJS.ErrnoException | undefined)?.code;
    return code === 'ENOENT' || code === 'ECONNREFUSED';
  }

  /** Core request primitive. The daemon's wire format is:
   * `{ method, params, project } \n`
   * on the way in, and the Response envelope \n on the way out.
   * `project` defaults to `process.cwd()`.
   *
   * On ENOENT/ECONNREFUSED, if `opts.retryOnRefuse` is true and
   * `opts.spawnFn` is provided, calls `spawnFn()`, waits
   * `opts.spawnDelayMs` (default 500ms), then retries exactly once.
   *
   * `timeoutMs` overrides the constructor default for this call only.
   * Pass a shorter value for always-fast methods (health, inspect) and
   * a longer one for cold-start reindex. */
  async request<T = unknown>(
    method: string,
    params: object,
    project?: string,
    timeoutMs?: number,
  ): Promise<T> {
    const effectiveTimeout = timeoutMs ?? this.timeoutMs;
    try {
      const result = await this.sendRequest<T>(
        method,
        params,
        project,
        effectiveTimeout,
      );
      this.fireSuccess(method);
      return result;
    } catch (e: unknown) {
      if (this.shouldRetry(e) && this.opts.spawnFn) {
        // First attempt failed with a retryable connection error —
        // surface it to observers before we try the spawn path so
        // UI can flip to "starting".
        this.fireFailed(
          method,
          e instanceof Error ? e : new Error(String(e)),
        );
        // Dedupe concurrent spawns: first caller actually invokes
        // spawnFn; siblings await the same promise. Without this,
        // N providers hitting ENOENT at activation spawn N daemons.
        if (!this.spawnInFlight) {
          this.spawnInFlight = (async () => {
            try {
              await this.opts.spawnFn!();
              await delay(this.opts.spawnDelayMs ?? 500);
            } finally {
              this.spawnInFlight = null;
            }
          })();
        }
        await this.spawnInFlight;
        try {
          const retryResult = await this.sendRequest<T>(
            method,
            params,
            project,
            effectiveTimeout,
          );
          this.fireSuccess(method);
          return retryResult;
        } catch (retryErr: unknown) {
          this.fireFailed(
            method,
            retryErr instanceof Error
              ? retryErr
              : new Error(String(retryErr)),
          );
          throw retryErr;
        }
      }
      this.fireFailed(method, e instanceof Error ? e : new Error(String(e)));
      throw e;
    }
  }

  /** Low-level send: opens a socket, writes the request, reads one
   * newline-delimited response, and closes. No retry logic here. */
  private sendRequest<T>(
    method: string,
    params: object,
    project: string | undefined,
    timeoutMs: number,
  ): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const sock = net.createConnection(this.socketPath);
      let buf = '';
      let settled = false;

      const timeout = setTimeout(() => {
        if (settled) return;
        settled = true;
        sock.destroy();
        reject(
          new CoreGraphError(
            `IPC timeout for '${method}' after ${timeoutMs}ms`,
          ),
        );
      }, timeoutMs);

      const settle = (fn: () => void) => {
        if (settled) return;
        settled = true;
        clearTimeout(timeout);
        fn();
      };

      sock.on('connect', () => {
        const req = JSON.stringify({
          method,
          params,
          project: project ?? process.cwd(),
        });
        sock.write(req + '\n');
      });

      sock.on('data', (chunk) => {
        if (settled) return;
        buf += chunk.toString();
        const idx = buf.indexOf('\n');
        if (idx === -1) return;
        const line = buf.slice(0, idx);
        try {
          const resp: Response = JSON.parse(line);
          sock.end();
          if (!resp.ok) {
            settle(() =>
              reject(new CoreGraphError(resp.error ?? 'unknown IPC error')),
            );
            return;
          }
          // Body is a JSON-encoded string. Parse it before handing back.
          const body = JSON.parse(resp.body) as T;
          settle(() => resolve(body));
        } catch (e: unknown) {
          settle(() => reject(e));
        }
      });

      sock.on('error', (e: Error) => {
        settle(() => reject(e));
      });

      sock.on('close', () => {
        settle(() =>
          reject(
            new CoreGraphError(
              `IPC connection closed before '${method}' response`,
            ),
          ),
        );
      });
    });
  }

  /** `health` — daemon liveness. */
  health(timeoutMs?: number): Promise<HealthResponse> {
    return this.request<HealthResponse>('health', {}, undefined, timeoutMs);
  }

  /** `reindex` — trigger a graph rebuild. mode='fast' requires a file.
   *
   * `project` overrides the project root sent in the IPC envelope.
   * When omitted, `process.cwd()` is used (the default from `request()`).
   * Pass the project root explicitly in integration tests so the daemon
   * operates on the correct graph rather than the test runner's CWD. */
  reindex(
    file: string | undefined,
    mode: 'fast' | 'full' = 'full',
    project?: string,
    timeoutMs?: number,
  ): Promise<ReindexResult> {
    // Surface the daemon's invariant on the client to fail fast without a
    // round-trip: mode='fast' is a per-file incremental rebuild and has
    // no meaning without a target file.
    if (mode === 'fast' && file === undefined) {
      return Promise.reject(
        new CoreGraphError("reindex: mode='fast' requires a file path"),
      );
    }
    const params: { mode: 'fast' | 'full'; file?: string } = { mode };
    if (file !== undefined) params.file = file;
    return this.request<ReindexResult>('reindex', params, project, timeoutMs);
  }

  /** `inspect` — resolve a file/line/column to a symbol + edges.
   *
   * `project` overrides the project root in the IPC envelope. Pass it
   * explicitly when the caller's CWD is not the workspace root (e.g.
   * the VSCode extension where each workspace folder is a separate
   * project). When omitted, `process.cwd()` is used. */
  inspect(
    file: string,
    line: number,
    column: number,
    project?: string,
    timeoutMs?: number,
  ): Promise<InspectResult> {
    return this.request<InspectResult>(
      'inspect',
      { file, line, column },
      project,
      timeoutMs,
    );
  }

  /** Drop all event subscribers. Safe to call on extension deactivation
   * even though the socket lifecycle is stateless (no connections to
   * close). */
  dispose(): void {
    this.successListeners.length = 0;
    this.failedListeners.length = 0;
  }
}
