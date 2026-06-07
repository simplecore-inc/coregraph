import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type { DiffResult } from "../ipc/types";
import { buildReviewMarkdown } from "./reviewMarkdown";
import { markdownToHtml } from "./markdownToHtml";

/** Singleton webview panel for the "Preview Review Comment" command.
 *
 * Fetches the current working-tree diff via `ipc.diff`, generates a
 * Markdown comment body that mirrors the style of `coregraph review`'s
 * `render_comment`, renders it to HTML, and displays it in a webview.
 *
 * Copy button posts the original Markdown (not the rendered HTML) to
 * the clipboard. Refresh button triggers a fresh `ipc.diff` call. */
export class ReviewPreviewPanel {
  private static current: ReviewPreviewPanel | undefined;

  private readonly panel: vscode.WebviewPanel;
  private readonly disposables: vscode.Disposable[] = [];
  private lastMarkdown: string = "";

  private constructor(
    panel: vscode.WebviewPanel,
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: () => string | undefined,
  ) {
    this.panel = panel;
    this.disposables.push(
      this.panel.onDidDispose(() => this.dispose()),
      this.panel.webview.onDidReceiveMessage((msg: unknown) => {
        if (
          msg !== null &&
          typeof msg === "object" &&
          (msg as Record<string, unknown>)["type"] === "copy"
        ) {
          void vscode.env.clipboard.writeText(this.lastMarkdown);
          void vscode.window.showInformationMessage(
            "CoreGraph: Review comment copied to clipboard.",
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
  ): ReviewPreviewPanel {
    const column =
      vscode.window.activeTextEditor?.viewColumn ?? vscode.ViewColumn.One;
    if (ReviewPreviewPanel.current) {
      ReviewPreviewPanel.current.panel.reveal(column);
      void ReviewPreviewPanel.current.refresh();
      return ReviewPreviewPanel.current;
    }
    const panel = vscode.window.createWebviewPanel(
      "coregraph.reviewPreview",
      "CoreGraph Review Preview",
      column,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
      },
    );
    ReviewPreviewPanel.current = new ReviewPreviewPanel(
      panel,
      ipc,
      projectRootResolver,
    );
    void ReviewPreviewPanel.current.refresh();
    return ReviewPreviewPanel.current;
  }

  /** Fetch a fresh diff from the daemon and update the webview content. */
  public async refresh(): Promise<void> {
    const project = this.projectRootResolver();
    if (!project) {
      this.panel.webview.html = this.wrapHtml(
        "<p><em>No workspace folder is open.</em></p>",
        "(no workspace)",
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
      this.panel.webview.html = this.wrapHtml(
        `<p><em>Failed to fetch diff: ${msg.replace(/</g, "&lt;")}</em></p>`,
        "(fetch failed)",
      );
      return;
    }
    this.lastMarkdown = buildReviewMarkdown(diff);
    const renderedHtml = markdownToHtml(this.lastMarkdown);
    this.panel.webview.html = this.wrapHtml(renderedHtml, diff.base_ref);
  }

  /** Wrap rendered content with the panel chrome (copy button, CSP,
   * theming CSS). `subtitle` is shown after the main heading. */
  private wrapHtml(body: string, subtitle: string): string {
    const csp = this.panel.webview.cspSource;
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy"
  content="default-src 'none'; style-src ${csp} 'unsafe-inline'; script-src ${csp} 'unsafe-inline';">
<title>CoreGraph Review Preview</title>
<style>
  body { font-family: var(--vscode-font-family); color: var(--vscode-foreground);
         background: var(--vscode-editor-background); padding: 1rem 2rem; line-height: 1.5; }
  .chrome { display: flex; justify-content: space-between; align-items: center;
            margin-bottom: 1rem; border-bottom: 1px solid var(--vscode-panel-border); padding-bottom: 0.5rem; }
  .subtitle { color: var(--vscode-descriptionForeground); font-size: 0.85rem; }
  button { background: var(--vscode-button-background); color: var(--vscode-button-foreground);
           border: none; padding: 0.4rem 0.8rem; cursor: pointer; border-radius: 3px; }
  button:hover { background: var(--vscode-button-hoverBackground); }
  .rendered h2 { font-size: 1.2rem; }
  .rendered h3 { font-size: 1.0rem; }
  .rendered table { border-collapse: collapse; }
  .rendered th, .rendered td {
    border: 1px solid var(--vscode-panel-border); padding: 0.3rem 0.6rem;
  }
  .rendered th { background: var(--vscode-textCodeBlock-background); }
  .rendered code { font-family: var(--vscode-editor-font-family);
                   background: var(--vscode-textCodeBlock-background);
                   padding: 0 0.3rem; border-radius: 3px; }
  .rendered blockquote { color: var(--vscode-descriptionForeground);
                         border-left: 3px solid var(--vscode-panel-border);
                         margin-left: 0; padding-left: 1em; }
</style>
</head>
<body>
  <div class="chrome">
    <div class="subtitle">Preview against <code>${subtitle.replace(/</g, "&lt;")}</code></div>
    <div>
      <button id="refresh">Refresh</button>
      <button id="copy">Copy Markdown</button>
    </div>
  </div>
  <div class="rendered">${body}</div>
  <script>
    const vscode = acquireVsCodeApi();
    document.getElementById("copy").addEventListener("click", () => {
      vscode.postMessage({ type: "copy" });
    });
    document.getElementById("refresh").addEventListener("click", () => {
      vscode.postMessage({ type: "refresh" });
    });
  </script>
</body>
</html>`;
  }

  public dispose(): void {
    ReviewPreviewPanel.current = undefined;
    for (const d of this.disposables) d.dispose();
    this.panel.dispose();
  }
}
