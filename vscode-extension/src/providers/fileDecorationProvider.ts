import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type { DiffResult, DiffFileImpact } from "../ipc/types";
import {
  impactTier,
  themeColorId,
  badgeChar,
  formatTooltip,
  toDecorationMap,
} from "./fileDecorationFormat";

export class CoreGraphFileDecorationProvider
  implements vscode.FileDecorationProvider {
  private readonly _onDidChange = new vscode.EventEmitter<
    vscode.Uri[] | undefined
  >();
  public readonly onDidChangeFileDecorations = this._onDidChange.event;

  private decorations = new Map<string, DiffFileImpact>();
  /** True while a diff fetch is in flight — prevents concurrent fetches
   * from racing. The next save/git event will re-trigger after it settles. */
  private fetching = false;

  constructor(
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: () => string | undefined,
  ) {}

  public provideFileDecoration(
    uri: vscode.Uri,
  ): vscode.FileDecoration | undefined {
    if (uri.scheme !== "file") return undefined;
    const hit = this.decorations.get(uri.fsPath);
    if (!hit) return undefined;
    const tier = impactTier(hit.confidence_weighted);
    const deco = new vscode.FileDecoration(
      badgeChar(tier),
      formatTooltip(hit),
      new vscode.ThemeColor(themeColorId(tier)),
    );
    // Propagate the decoration up to parent directories so the Explorer
    // roll-up badge is also visible alongside the SCM panel.
    deco.propagate = true;
    return deco;
  }

  /** Re-fetch the diff from the daemon and refresh decorations.
   * Called on save, git events, or explicit user commands. Concurrent
   * calls are coalesced — only one fetch runs at a time. */
  public async refresh(): Promise<void> {
    if (this.fetching) return;
    const project = this.projectRootResolver();
    if (!project) {
      this.decorations = new Map();
      this._onDidChange.fire(undefined);
      return;
    }

    this.fetching = true;
    try {
      const diff = await this.ipc.request<DiffResult>(
        "diff",
        { base_ref: "HEAD" },
        project,
      );
      // Fire the union of old + new URIs so VSCode refreshes both removed
      // decorations and newly added ones — otherwise stale badges linger.
      const oldUris = [...this.decorations.keys()];
      const nextMap = toDecorationMap(diff);
      const newUris = [...nextMap.keys()];
      this.decorations = nextMap;
      const allPaths = new Set([...oldUris, ...newUris]);
      this._onDidChange.fire(
        [...allPaths].map((p) => vscode.Uri.file(p)),
      );
    } catch (e) {
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      console.debug(`CoreGraph: SCM decoration refresh failed: ${msg}`);
      // On IPC failure keep existing decorations — stale is better than blank.
    } finally {
      this.fetching = false;
    }
  }

  /** Remove all decorations immediately (e.g. on deactivation or project close). */
  public clear(): void {
    const oldUris = [...this.decorations.keys()];
    this.decorations = new Map();
    this._onDidChange.fire(oldUris.map((p) => vscode.Uri.file(p)));
  }

  public dispose(): void {
    this._onDidChange.dispose();
  }
}
