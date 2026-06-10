import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type { DiffResult } from "../ipc/types";
import { type WarnThresholds, decideWarning } from "./commitWarningFormat";
import type { DaemonHealthTracker } from "../util/daemonHealth";

export type { WarnThresholds };
export { decideWarning };

/** Read thresholds from workspace config; defaults per spec §6.3. */
export function readThresholds(): WarnThresholds {
  const cfg = vscode.workspace.getConfiguration("coregraph");
  return {
    enabled: cfg.get<boolean>("warnOnCommit.enabled", false),
    impactThreshold: cfg.get<number>("warnOnCommit.impactThreshold", 20),
    inconsistencyCount: cfg.get<number>("warnOnCommit.inconsistencyCount", 1),
  };
}

export class CoreGraphCommitWarningProvider {
  private readonly statusBar: vscode.StatusBarItem;
  private fetching = false;
  /** The repo's original commit-input placeholder, captured the first time we
   * overwrite it so `clearScmPlaceholder` restores it instead of a fixed
   * English default that would clobber the user's locale/keybinding hint. */
  private savedPlaceholder: string | undefined;
  private placeholderCaptured = false;

  constructor(
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: () => string | undefined,
    private readonly tracker: DaemonHealthTracker,
  ) {
    this.statusBar = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Right,
      1000, // priority — to the left of most other right-aligned items
    );
    this.statusBar.command = "coregraph.showDiffImpact";
    this.statusBar.hide();
  }

  /** Subscribe to configuration changes and register disposables.
   * Call from `activate`. */
  public attach(context: vscode.ExtensionContext): void {
    // Subscribe to daemon health state changes.
    const trackerDisposable = this.tracker.onDidChangeState((state) => {
      if (state === "offline") {
        this.statusBar.hide();
      } else if (state === "online") {
        void this.refresh();
      }
    });

    context.subscriptions.push(
      this.statusBar,
      trackerDisposable,
      vscode.workspace.onDidChangeConfiguration((e) => {
        if (e.affectsConfiguration("coregraph.warnOnCommit")) {
          void this.refresh();
        }
      }),
      { dispose: () => this.dispose() },
    );

    // Render initial state.
    if (this.tracker.state === "offline") {
      this.statusBar.hide();
    }
  }

  /** Re-fetch diff and update status bar + SCM input placeholder. */
  public async refresh(): Promise<void> {
    if (this.fetching) return;
    if (this.tracker.state === "offline") {
      this.statusBar.hide();
      return;
    }
    const thresholds = readThresholds();
    if (!thresholds.enabled) {
      this.statusBar.hide();
      this.clearScmPlaceholder();
      return;
    }
    const project = this.projectRootResolver();
    if (!project) {
      this.statusBar.hide();
      return;
    }

    this.fetching = true;
    try {
      const diff = await this.ipc.request<DiffResult>(
        "diff",
        { base_ref: "HEAD" },
        project,
      );
      const warning = decideWarning(diff, thresholds);
      if (warning === null) {
        this.statusBar.hide();
        this.clearScmPlaceholder();
      } else {
        this.statusBar.text = warning;
        this.statusBar.backgroundColor = new vscode.ThemeColor(
          "statusBarItem.warningBackground",
        );
        this.statusBar.tooltip = new vscode.MarkdownString(
          `**${warning}**\n\n` +
            `Changed files: ${diff.changed_files.length}\n` +
            `Reach: ${diff.total_reachable}\n\n` +
            `Click to open Diff Impact panel.`,
        );
        this.statusBar.show();
        this.setScmPlaceholder(warning);
      }
    } catch (e) {
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      console.debug(`CoreGraph: commit-warning refresh failed: ${msg}`);
    } finally {
      this.fetching = false;
    }
  }

  /** Inject the warning into the active repository's commit input
   * placeholder, if the built-in git API is reachable. Best-effort —
   * if the vscode.git extension isn't loaded, silently skip. */
  private setScmPlaceholder(message: string): void {
    try {
      const gitExt = vscode.extensions.getExtension("vscode.git");
      if (!gitExt?.isActive) return;
      // vscode.git public API v1: getAPI(1).repositories[0].inputBox
      const api = gitExt.exports?.getAPI?.(1);
      const repo = api?.repositories?.[0];
      if (repo?.inputBox) {
        // Capture the original placeholder once, before our first overwrite,
        // so we can restore exactly what the user (or vscode) had.
        if (!this.placeholderCaptured) {
          this.savedPlaceholder = repo.inputBox.placeholder;
          this.placeholderCaptured = true;
        }
        repo.inputBox.placeholder = message;
      }
    } catch {
      // Silent — best-effort integration with a foreign extension.
    }
  }

  /** Restore the commit input placeholder to whatever it was before we first
   * overwrote it (captured in `setScmPlaceholder`). Falls back to leaving the
   * current value untouched if we never captured one, so we never clobber the
   * user's platform/keybinding/locale hint with a fixed English default. */
  private clearScmPlaceholder(): void {
    try {
      const gitExt = vscode.extensions.getExtension("vscode.git");
      if (!gitExt?.isActive) return;
      const api = gitExt.exports?.getAPI?.(1);
      const repo = api?.repositories?.[0];
      if (repo?.inputBox && this.placeholderCaptured) {
        repo.inputBox.placeholder = this.savedPlaceholder ?? "";
        this.placeholderCaptured = false;
        this.savedPlaceholder = undefined;
      }
    } catch {
      /* silent */
    }
  }

  public dispose(): void {
    this.clearScmPlaceholder();
    this.statusBar.dispose();
  }
}
