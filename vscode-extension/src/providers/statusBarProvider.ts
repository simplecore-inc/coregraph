import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type {
  InspectResult,
  ImpactBatchResult,
  ImpactEntry,
} from "../ipc/types";
import { formatStatusBar } from "./statusBarFormat";
import type { DaemonHealthTracker } from "../util/daemonHealth";

/** Debounce window between cursor movements before we issue an IPC call. */
const SELECTION_DEBOUNCE_MS = 200;

/** TTL for the position cache (gates ipc.inspect calls). */
const POSITION_CACHE_TTL_MS = 500;

/** TTL for the symbol-keyed impact cache (gates impact_batch calls). */
const SYMBOL_CACHE_TTL_MS = 500;

type PositionKey = string; // `${file}|${line}|${col}`
type SymbolKey = string;   // `${file}|${symbolName}`

interface PositionCacheEntry {
  at: number;
  inspect: InspectResult;
}

interface SymbolCacheEntry {
  at: number;
  inspect: InspectResult;
  impact: ImpactEntry | null;
}

export class CoreGraphStatusBarProvider {
  private readonly item: vscode.StatusBarItem;
  private readonly positionCache = new Map<PositionKey, PositionCacheEntry>();
  private readonly symbolCache = new Map<SymbolKey, SymbolCacheEntry>();
  private debounceTimer: NodeJS.Timeout | undefined;

  constructor(
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: (doc: vscode.TextDocument) => string,
    private readonly tracker: DaemonHealthTracker,
  ) {
    this.item = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Left,
      100,
    );
    this.item.command = "coregraph.statusBarClick";
    this.item.hide();
  }

  /** Subscribe to selection changes and daemon health. Call from `activate`. */
  public attach(context: vscode.ExtensionContext): void {
    const trackerDisposable = this.tracker.onDidChangeState((state) => {
      this.renderTrackerState(state);
    });

    context.subscriptions.push(
      vscode.window.onDidChangeTextEditorSelection((e) => {
        this.scheduleUpdate(e.textEditor);
      }),
      vscode.window.onDidChangeActiveTextEditor((editor) => {
        if (editor) {
          this.scheduleUpdate(editor);
        } else {
          this.item.hide();
        }
      }),
      trackerDisposable,
      this.item,
      { dispose: () => this.dispose() },
    );

    // Render initial tracker state when offline.
    if (this.tracker.state !== "online") {
      this.renderTrackerState(this.tracker.state);
    }

    // Initial render if an editor is already active.
    if (vscode.window.activeTextEditor && this.tracker.state === "online") {
      this.scheduleUpdate(vscode.window.activeTextEditor);
    }
  }

  private renderTrackerState(state: "starting" | "online" | "offline"): void {
    switch (state) {
      case "offline":
        this.item.text = "$(warning) CoreGraph offline";
        this.item.tooltip = "CoreGraph daemon is not running. Click to restart, or run CoreGraph: Restart Daemon from the Command Palette.";
        this.item.command = "coregraph.daemonRestart";
        this.item.show();
        break;
      case "starting":
        this.item.text = "$(loading~spin) CoreGraph: building graph\u2026";
        this.item.tooltip = "CoreGraph is indexing the workspace\u2026";
        this.item.show();
        break;
      case "online":
        // Resume cursor-tracking render.
        if (vscode.window.activeTextEditor) {
          this.scheduleUpdate(vscode.window.activeTextEditor);
        } else {
          this.item.hide();
        }
        break;
    }
  }

  private scheduleUpdate(editor: vscode.TextEditor): void {
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = undefined;
      void this.update(editor);
    }, SELECTION_DEBOUNCE_MS);
  }

  private async update(editor: vscode.TextEditor): Promise<void> {
    if (this.tracker.state !== "online") return;
    if (editor.document.uri.scheme !== "file") {
      this.item.hide();
      return;
    }
    const file = editor.document.uri.fsPath;
    const sel = editor.selection.active;
    const project = this.projectRootResolver(editor.document);

    // Position cache gates the inspect call.
    const posKey: PositionKey = `${file}|${sel.line}|${sel.character}`;
    const posCached = this.positionCache.get(posKey);
    let inspect: InspectResult;
    if (posCached && Date.now() - posCached.at < POSITION_CACHE_TTL_MS) {
      inspect = posCached.inspect;
    } else {
      try {
        inspect = await this.ipc.inspect(file, sel.line, sel.character, project);
        this.positionCache.set(posKey, { at: Date.now(), inspect });
      } catch (e) {
        const msg = e instanceof CoreGraphError ? e.message : String(e);
        console.debug(`CoreGraph: status-bar inspect failed: ${msg}`);
        this.item.hide();
        return;
      }
    }

    if (inspect.symbol === null) {
      this.item.hide();
      return;
    }

    const symbolName = inspect.symbol.name;
    const symKey: SymbolKey = `${file}|${symbolName}`;
    const symCached = this.symbolCache.get(symKey);
    let impact: ImpactEntry | null;
    if (symCached && Date.now() - symCached.at < SYMBOL_CACHE_TTL_MS) {
      impact = symCached.impact;
    } else {
      impact = await this.fetchImpact(symbolName, project);
      this.symbolCache.set(symKey, { at: Date.now(), inspect, impact });
    }

    const render = formatStatusBar(inspect, impact);
    if (!render.visible) {
      this.item.hide();
      return;
    }
    this.item.text = `$(pulse) ${render.text}`;
    // Restore the normal click target: an earlier offline episode switched
    // this to `coregraph.daemonRestart`, and without resetting it here every
    // later click on the live readout would re-open the restart prompt.
    this.item.command = "coregraph.statusBarClick";
    this.item.tooltip = render.tooltip;
    this.item.accessibilityInformation = {
      label: render.accessibleText,
      role: "button",
    };
    this.item.show();
  }

  private async fetchImpact(
    name: string,
    project: string,
  ): Promise<ImpactEntry | null> {
    try {
      const body = await this.ipc.request<ImpactBatchResult>(
        "impact_batch",
        { symbols: [name] },
        project,
      );
      const entry = body.results[name];
      return entry ?? null;
    } catch (e) {
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      console.debug(`CoreGraph: status-bar impact_batch failed: ${msg}`);
      return null;
    }
  }

  /** Drop all cached entries (e.g. on save → reindex). */
  public invalidateAll(): void {
    this.positionCache.clear();
    this.symbolCache.clear();
  }

  public dispose(): void {
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    this.positionCache.clear();
    this.symbolCache.clear();
    this.item.dispose();
  }
}
