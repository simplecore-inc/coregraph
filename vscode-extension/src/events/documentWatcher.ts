import * as vscode from 'vscode';
import { IpcClient, CoreGraphError } from '../ipc/client';
import { PathDebouncer } from '../util/debouncer';

export interface WatcherOptions {
  /** Milliseconds of quiet time before a change triggers a fast reindex.
   * Default 300. */
  debounceMs?: number;
  /** Optional tap called every time a fast reindex is dispatched — used
   * by tests/telemetry. */
  onFast?: (file: string) => void;
  /** Optional tap called on every full reindex. Kept for API compatibility;
   * full reindex is now triggered from extension.ts save handler. */
  onFull?: (file: string) => void;
  /** Predicate to decide which documents the watcher should handle.
   * The extension passes a language-filter here so settings.json, output
   * panel buffers, etc. don't trigger reindex of arbitrary directories.
   * Default: accept every file:// document. */
  shouldHandle?: (doc: vscode.TextDocument) => boolean;
}

/**
 * Register document-event listeners that keep the daemon's graph in
 * sync with editor state:
 *   - `onDidChangeTextDocument` triggers a debounced fast reindex.
 *
 * Save-triggered full reindex has been moved to the extension.ts save
 * handler (IC-8) so that providers can be invalidated in the correct
 * order after reindex completes.
 *
 * The debouncer is disposed so pending timers don't fire after extension
 * deactivate.
 */
export function registerDocumentWatcher(
  ctx: vscode.ExtensionContext,
  ipc: IpcClient,
  opts: WatcherOptions = {},
): void {
  const debouncer = new PathDebouncer(opts.debounceMs ?? 300);
  const shouldHandle = opts.shouldHandle ?? (() => true);

  const changeDisposable = vscode.workspace.onDidChangeTextDocument((e) => {
    if (e.document.uri.scheme !== "file" || !shouldHandle(e.document)) return;
    const file = e.document.uri.fsPath;
    debouncer.schedule(file, async () => {
      try {
        await ipc.reindex(file, 'fast', undefined, 10_000);
        opts.onFast?.(file);
      } catch (err) {
        // Fast reindex is best-effort — log at debug, don't surface.
        const msg = err instanceof CoreGraphError ? err.message : String(err);
        console.debug(`CoreGraph: fast reindex dropped for ${file}: ${msg}`);
      }
    });
  });

  ctx.subscriptions.push(
    changeDisposable,
    { dispose: () => debouncer.cancelAll() },
  );
}
