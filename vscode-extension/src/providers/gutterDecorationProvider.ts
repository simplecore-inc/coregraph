import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type { DiffResult } from "../ipc/types";
import {
  toGutterHits,
  formatGutterTooltip,
  type GutterHit,
} from "./gutterDecorationFormat";
import { impactTier } from "./fileDecorationFormat";

export class CoreGraphGutterDecorationProvider {
  private readonly lowType: vscode.TextEditorDecorationType;
  private readonly mediumType: vscode.TextEditorDecorationType;
  private readonly highType: vscode.TextEditorDecorationType;
  private hitsByFile = new Map<string, GutterHit[]>();
  private fetching = false;

  constructor(
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: () => string | undefined,
  ) {
    this.lowType = vscode.window.createTextEditorDecorationType({
      before: {
        contentText: "\u2022",
        color: new vscode.ThemeColor("coregraph.impact.low"),
        margin: "0 0.5em 0 0",
      },
      overviewRulerColor: new vscode.ThemeColor("coregraph.impact.low"),
      overviewRulerLane: vscode.OverviewRulerLane.Right,
    });

    this.mediumType = vscode.window.createTextEditorDecorationType({
      before: {
        contentText: "!",
        color: new vscode.ThemeColor("coregraph.impact.medium"),
        margin: "0 0.5em 0 0",
      },
      overviewRulerColor: new vscode.ThemeColor("coregraph.impact.medium"),
      overviewRulerLane: vscode.OverviewRulerLane.Right,
    });

    this.highType = vscode.window.createTextEditorDecorationType({
      before: {
        contentText: "\u203C",
        color: new vscode.ThemeColor("coregraph.impact.high"),
        margin: "0 0.5em 0 0",
      },
      overviewRulerColor: new vscode.ThemeColor("coregraph.impact.high"),
      overviewRulerLane: vscode.OverviewRulerLane.Right,
    });
  }

  /** Register event subscriptions and push disposables into `context`. */
  public attach(context: vscode.ExtensionContext): void {
    context.subscriptions.push(
      this.lowType,
      this.mediumType,
      this.highType,
      vscode.window.onDidChangeActiveTextEditor((ed) => {
        if (ed) this.applyTo(ed);
      }),
      { dispose: () => this.dispose() },
    );
  }

  /** Re-fetch the diff from the daemon and apply decorations to all visible
   * editors. Concurrent calls are coalesced via `fetching`. */
  public async refresh(): Promise<void> {
    if (this.fetching) return;
    const project = this.projectRootResolver();
    if (!project) {
      this.hitsByFile = new Map();
      return;
    }
    this.fetching = true;
    try {
      const diff = await this.ipc.request<DiffResult>(
        "diff",
        { base_ref: "HEAD" },
        project,
      );
      const next = new Map<string, GutterHit[]>();
      for (const h of toGutterHits(diff)) {
        const arr = next.get(h.file);
        if (arr) arr.push(h);
        else next.set(h.file, [h]);
      }
      this.hitsByFile = next;
      for (const ed of vscode.window.visibleTextEditors) {
        this.applyTo(ed);
      }
    } catch (e) {
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      console.debug(`CoreGraph: gutter refresh failed: ${msg}`);
    } finally {
      this.fetching = false;
    }
  }

  /** Apply stored decorations for the given editor's file. */
  private applyTo(editor: vscode.TextEditor): void {
    if (editor.document.uri.scheme !== "file") return;
    const hits = this.hitsByFile.get(editor.document.uri.fsPath) ?? [];
    const threshold = vscode.workspace
      .getConfiguration("coregraph")
      .get<number>("warnOnCommit.impactThreshold", 20);

    const lowHits: vscode.DecorationOptions[] = [];
    const mediumHits: vscode.DecorationOptions[] = [];
    const highHits: vscode.DecorationOptions[] = [];

    for (const h of hits) {
      const decoration: vscode.DecorationOptions = {
        range: new vscode.Range(h.line, 0, h.line, 0),
        hoverMessage: new vscode.MarkdownString(formatGutterTooltip(h, threshold)),
      };
      const tier = impactTier(h.confidence_weighted);
      if (tier === "high") highHits.push(decoration);
      else if (tier === "medium") mediumHits.push(decoration);
      else lowHits.push(decoration);
    }

    editor.setDecorations(this.lowType, lowHits);
    editor.setDecorations(this.mediumType, mediumHits);
    editor.setDecorations(this.highType, highHits);
  }

  /** Clear all stored hits (called on dispose). */
  public dispose(): void {
    this.hitsByFile = new Map();
  }
}
