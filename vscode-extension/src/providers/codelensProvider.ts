import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type { ImpactBatchResult, ImpactEntry } from "../ipc/types";
import {
  flattenAndFilterDecls,
  formatTitle,
  formatCodeLensTooltip,
  type SymbolLike,
} from "./codelensFormat";
import type { DocumentSymbolCache } from "../util/documentSymbolCache";
import { LRUMap } from "../util/lruMap";

/** Capacity matches the Rust-side `MAX_SYMBOLS_PER_BATCH` (64). */
const MAX_SYMBOLS_PER_REQUEST = 64;

/** Cache TTL: 5 seconds. */
const CACHE_TTL_MS = 5000;

interface CacheEntry {
  at: number;
  data: Map<string, ImpactEntry | null>;
}

export class CoreGraphCodeLensProvider implements vscode.CodeLensProvider {
  private readonly cache = new LRUMap<string, CacheEntry>(64);
  private readonly shownEmptyToasts = new Set<string>();

  private readonly _onDidChange = new vscode.EventEmitter<void>();
  public readonly onDidChangeCodeLenses = this._onDidChange.event;

  constructor(
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: (doc: vscode.TextDocument) => string,
    private readonly symbolCache: DocumentSymbolCache,
  ) {}

  /** Called by VSCode on every refresh. */
  async provideCodeLenses(
    document: vscode.TextDocument,
    token: vscode.CancellationToken,
  ): Promise<vscode.CodeLens[]> {
    const docSymbols = await this.symbolCache.get(document);
    if (token.isCancellationRequested) return [];
    if (docSymbols.length === 0) return [];

    const decls = flattenAndFilterDecls<vscode.Range>(
      docSymbols as SymbolLike<vscode.Range>[],
    ).slice(0, MAX_SYMBOLS_PER_REQUEST);
    if (decls.length === 0) return [];

    const names = decls.map((d) => d.name);
    const project = this.projectRootResolver(document);

    const impactMap = await this.getImpactWithCache(
      document.uri.toString(),
      names,
      project,
    );
    if (token.isCancellationRequested) return [];

    // Fire a one-time toast when IPC returned data but every entry is null
    // (graph exists but none of these symbols are tracked yet).
    if (
      impactMap.size > 0 &&
      [...impactMap.values()].every((v) => v === null) &&
      !this.shownEmptyToasts.has(document.uri.toString())
    ) {
      this.shownEmptyToasts.add(document.uri.toString());
      void vscode.window.showInformationMessage(
        "CoreGraph: no graph data yet for this file. Save the file or run CoreGraph: Restart Daemon from the Command Palette to trigger indexing.",
      );
    }

    const threshold = vscode.workspace
      .getConfiguration("coregraph")
      .get<number>("warnOnCommit.impactThreshold", 20);

    return decls
      .map((decl) => {
        const entry = impactMap.get(decl.name);
        if (entry === undefined) return null;
        return makeLens(decl.range, decl.name, entry, document.uri, threshold);
      })
      .filter((l): l is vscode.CodeLens => l !== null);
  }

  private async getImpactWithCache(
    uri: string,
    names: string[],
    project: string,
  ): Promise<Map<string, ImpactEntry | null>> {
    const cached = this.cache.get(uri);
    if (cached && Date.now() - cached.at < CACHE_TTL_MS) {
      const missing = names.some((n) => !cached.data.has(n));
      if (!missing) return cached.data;
    }

    try {
      const body = await this.ipc.request<ImpactBatchResult>(
        "impact_batch",
        { symbols: names },
        project,
      );
      const map = new Map<string, ImpactEntry | null>();
      for (const [name, entry] of Object.entries(body.results)) {
        map.set(name, entry);
      }
      this.cache.set(uri, { at: Date.now(), data: map });
      return map;
    } catch (e) {
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      console.debug(`CoreGraph: CodeLens impact_batch failed: ${msg}`);
      return new Map();
    }
  }

  /** Invalidate the CodeLens cache for a specific file. */
  public invalidate(uri: vscode.Uri): void {
    this.cache.delete(uri.toString());
    this._onDidChange.fire();
  }

  /** Drop all cached entries. */
  public clear(): void {
    this.cache.clear();
  }
}

function makeLens(
  range: vscode.Range,
  name: string,
  entry: ImpactEntry | null,
  uri: vscode.Uri,
  threshold: number,
): vscode.CodeLens {
  const title = formatTitle(entry);
  const tooltip = formatCodeLensTooltip(entry, threshold);
  const command: vscode.Command =
    entry !== null && entry.nodes > 0
      ? {
          title,
          tooltip,
          command: "coregraph.showImpact",
          arguments: [uri, name, entry],
        }
      : {
          title,
          tooltip,
          command: "coregraph.codelensNoop",
          arguments: [],
        };
  return new vscode.CodeLens(range, command);
}
