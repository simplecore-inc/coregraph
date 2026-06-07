import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type { DiffResult } from "../ipc/types";
import { buildDiffImpactHtml } from "./diffImpactHtml";
import { diffResultToElements } from "./diffImpactGraph";

/** Singleton webview panel for the "Show Diff Impact" command.
 *
 * Only one panel exists at a time: invoking the command re-uses the
 * existing panel (revealing it) and refreshes its contents from a
 * fresh `ipc.diff` call. */
export class DiffImpactPanel {
  private static current: DiffImpactPanel | undefined;

  private readonly panel: vscode.WebviewPanel;
  private readonly disposables: vscode.Disposable[] = [];

  private constructor(
    panel: vscode.WebviewPanel,
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: () => string | undefined,
    private readonly extensionUri: vscode.Uri,
  ) {
    this.panel = panel;
    this.disposables.push(
      this.panel.onDidDispose(() => this.dispose()),
      this.panel.webview.onDidReceiveMessage((msg: unknown) => {
        if (
          msg !== null &&
          typeof msg === "object" &&
          (msg as Record<string, unknown>)["type"] === "openFile" &&
          typeof (msg as Record<string, unknown>)["file"] === "string"
        ) {
          const m = msg as Record<string, unknown>;
          void this.openFile(
            m["file"] as string,
            typeof m["line"] === "number" ? (m["line"] as number) : 0,
          );
        } else if (
          msg !== null &&
          typeof msg === "object" &&
          (msg as Record<string, unknown>)["type"] === "refresh"
        ) {
          void this.refresh();
        }
      }),
    );
  }

  /** Create a new panel or reveal and refresh the existing one. */
  public static createOrShow(
    ipc: IpcClient,
    projectRootResolver: () => string | undefined,
    extensionUri: vscode.Uri,
  ): DiffImpactPanel {
    const column =
      vscode.window.activeTextEditor?.viewColumn ?? vscode.ViewColumn.One;
    if (DiffImpactPanel.current) {
      DiffImpactPanel.current.panel.reveal(column);
      void DiffImpactPanel.current.refresh();
      return DiffImpactPanel.current;
    }
    const panel = vscode.window.createWebviewPanel(
      "coregraph.diffImpact",
      "CoreGraph Diff Impact",
      column,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        // Allow the webview to load the vendored cytoscape.min.js from
        // the extension's media directory.
        localResourceRoots: [vscode.Uri.joinPath(extensionUri, "media")],
      },
    );
    DiffImpactPanel.current = new DiffImpactPanel(
      panel,
      ipc,
      projectRootResolver,
      extensionUri,
    );
    void DiffImpactPanel.current.refresh();
    return DiffImpactPanel.current;
  }

  /** Fetch a fresh diff from the daemon and update the webview content. */
  public async refresh(): Promise<void> {
    const project = this.projectRootResolver();
    if (!project) {
      this.panel.webview.html = this.errorHtml(
        "CoreGraph: no workspace folder is open.",
      );
      return;
    }
    let diff: DiffResult;
    try {
      diff = await this.ipc.request<DiffResult>(
        "diff",
        { base_ref: "HEAD" },
        project,
      );
    } catch (e: unknown) {
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      this.panel.webview.html = this.errorHtml(`Failed to fetch diff: ${msg}`);
      return;
    }

    // Resolve the vendored Cytoscape bundle to a webview-safe URI.
    const cytoscapeUri = this.panel.webview
      .asWebviewUri(
        vscode.Uri.joinPath(this.extensionUri, "media", "cytoscape.min.js"),
      )
      .toString();

    // Pre-serialize the graph elements — pure conversion, no vscode import.
    // Escape '<' as \u003c so a literal '</script>' in a file path or symbol
    // name cannot terminate the enclosing <script> tag in the webview HTML.
    const elementsJson = JSON.stringify(diffResultToElements(diff)).replace(/</g, "\\u003c");

    this.panel.webview.html = buildDiffImpactHtml(
      diff,
      this.panel.webview.cspSource,
      cytoscapeUri,
      elementsJson,
    );
  }

  /** Open a source file in the editor and jump to the given line. */
  private async openFile(file: string, line: number): Promise<void> {
    try {
      const doc = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
      const editor = await vscode.window.showTextDocument(doc);
      const pos = new vscode.Position(Math.max(0, line), 0);
      editor.selection = new vscode.Selection(pos, pos);
      editor.revealRange(new vscode.Range(pos, pos));
    } catch {
      // Silent on failure — the file may have been deleted or moved.
    }
  }

  /** Render a minimal error page when IPC fails or no project is open. */
  private errorHtml(message: string): string {
    const esc = message.replace(/</g, "&lt;");
    return `<!DOCTYPE html><html><body style="font-family: var(--vscode-font-family); padding: 2rem;">
      <h2>CoreGraph Diff Impact</h2>
      <p>${esc}</p>
      </body></html>`;
  }

  public dispose(): void {
    DiffImpactPanel.current = undefined;
    for (const d of this.disposables) d.dispose();
    this.panel.dispose();
  }
}
