import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type { InspectResult } from "../ipc/types";
import { formatHoverMarkdown } from "./hoverFormat";
import { LRUMap } from "../util/lruMap";

/** 5-second LRU cache TTL — matches the Phase 0 §5.2 spec budget for
 * "subsequent hovers on the same symbol from a 5-second LRU cache". */
const CACHE_TTL_MS = 5000;

/** Cache key: `${file}|${line}|${col}`. The hover position is the
 * cursor cell, not the symbol identity, but VSCode coalesces hovers
 * over the same word so caching by position works in practice. */
type CacheKey = string;

interface CacheEntry {
  at: number;
  result: InspectResult;
}

export class CoreGraphHoverProvider implements vscode.HoverProvider {
  private readonly cache = new LRUMap<CacheKey, CacheEntry>(256);

  constructor(
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: (doc: vscode.TextDocument) => string,
  ) {}

  async provideHover(
    document: vscode.TextDocument,
    position: vscode.Position,
    token: vscode.CancellationToken,
  ): Promise<vscode.Hover | undefined> {
    if (token.isCancellationRequested) return undefined;

    const file = document.uri.fsPath;
    const key: CacheKey = `${file}|${position.line}|${position.character}`;

    const cached = this.cache.get(key);
    if (cached && Date.now() - cached.at < CACHE_TTL_MS) {
      return new vscode.Hover(toMarkdown(cached.result));
    }

    const project = this.projectRootResolver(document);
    let result: InspectResult;
    try {
      result = await this.ipc.inspect(
        file,
        position.line,
        position.character,
        project,
      );
    } catch (e) {
      // IPC failure → no hover. The editor falls back to other hover
      // providers (LSP) without our content.
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      console.debug(`CoreGraph: hover inspect failed: ${msg}`);
      return undefined;
    }

    if (token.isCancellationRequested) return undefined;

    // Suppress the hover entirely when there is no resolved symbol
    // AND no edges to surface — the empty card is noise.
    if (
      result.symbol === null &&
      result.edges_in.length === 0 &&
      result.edges_out.length === 0
    ) {
      return undefined;
    }

    this.cache.set(key, { at: Date.now(), result });
    return new vscode.Hover(toMarkdown(result));
  }

  /** Drop cached hovers for `uri`. Called on document save (after
   * watcher's reindex) so subsequent hovers see fresh data. */
  public invalidate(uri: vscode.Uri): void {
    const prefix = `${uri.fsPath}|`;
    for (const key of this.cache.keys()) {
      if (key.startsWith(prefix)) this.cache.delete(key);
    }
  }

  /** Drop everything. Called on disposal. */
  public clear(): void {
    this.cache.clear();
  }
}

function toMarkdown(result: InspectResult): vscode.MarkdownString {
  const md = new vscode.MarkdownString(formatHoverMarkdown(result));
  md.isTrusted = false; // no command links yet — Phase 1.4+ may add them
  md.supportHtml = false;
  return md;
}
