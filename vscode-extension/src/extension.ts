// CoreGraph VS Code extension entry point.
//
// Spawns `coregraph lsp` as the language server. The CLI binary
// implements the LSP stdio bridge (definition / references /
// workspace symbol) and routes to the CoreGraph daemon when
// available. We don't ship any in-extension intelligence — the
// extension is a pure transport that lets VS Code talk to whatever
// `coregraph` is on the user's $PATH.

import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";
import { spawn } from "node:child_process";
import { IpcClient } from "./ipc/client";
import { resolveSocketPath } from "./ipc/paths";
import { registerDocumentWatcher } from "./events/documentWatcher";
import { CoreGraphCodeLensProvider } from "./providers/codelensProvider";
import { CoreGraphHoverProvider } from "./providers/hoverProvider";
import { CoreGraphDiagnosticsProvider } from "./providers/diagnosticsProvider";
import { CoreGraphStatusBarProvider } from "./providers/statusBarProvider";
import { CoreGraphTreeProvider } from "./providers/treeProvider";
import { CoreGraphFileDecorationProvider } from "./providers/fileDecorationProvider";
import { CoreGraphGutterDecorationProvider } from "./providers/gutterDecorationProvider";
import { DiffImpactPanel } from "./panels/diffImpactPanel";
import { ReviewPreviewPanel } from "./panels/reviewPreviewPanel";
import { CoreGraphCommitWarningProvider } from "./providers/commitWarningProvider";
import { createDaemonHealthTracker } from "./util/daemonHealth";
import { createDocumentSymbolCache } from "./util/documentSymbolCache";

let client: LanguageClient | undefined;

// Languages CoreGraph extracts — shared with util/languageFilter for
// consistent filtering between extension.ts, documentWatcher, and tests.
import { SUPPORTED_LANGUAGES, isSupportedDoc as isSupportedDocHelper } from "./util/languageFilter";

const outputChannel = vscode.window.createOutputChannel("CoreGraph");

export function activate(context: vscode.ExtensionContext): void {
  const config = vscode.workspace.getConfiguration("coregraph");
  const binary = config.get<string>("binaryPath", "coregraph");

  // Resolve the LSP process CWD to the first workspace folder. Without
  // this, `vscode-languageclient` spawns `coregraph lsp` inheriting
  // VSCode's extension-host CWD — which on macOS defaults to `/` — so
  // the daemon's auto-spawned child (`server start`) treats the root
  // filesystem as the project root and hangs trying to index it.
  const lspCwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const serverOptions: ServerOptions = {
    command: binary,
    args: ["lsp"],
    transport: TransportKind.stdio,
    options: lspCwd ? { cwd: lspCwd } : undefined,
  };

  // Register against every language coregraph extracts. VSCode
  // multiplexes documents over a single LanguageClient when the
  // server agrees (which the coregraph LSP does — it ignores the
  // language ID, looks up symbols by name).
  const selector: vscode.DocumentSelector = [
    { scheme: "file", language: "rust" },
    { scheme: "file", language: "java" },
    { scheme: "file", language: "typescript" },
    { scheme: "file", language: "typescriptreact" },
    { scheme: "file", language: "javascript" },
    { scheme: "file", language: "javascriptreact" },
    { scheme: "file", language: "python" },
    { scheme: "file", language: "go" },
    { scheme: "file", language: "kotlin" },
  ];

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "rust" },
      { scheme: "file", language: "java" },
      { scheme: "file", language: "typescript" },
      { scheme: "file", language: "typescriptreact" },
      { scheme: "file", language: "javascript" },
      { scheme: "file", language: "javascriptreact" },
      { scheme: "file", language: "python" },
      { scheme: "file", language: "go" },
      { scheme: "file", language: "kotlin" },
    ],
    synchronize: {
      // Re-trigger on .coregraph/config.toml edits so users can
      // adjust exclude patterns without reloading.
      fileEvents: vscode.workspace.createFileSystemWatcher(
        "**/.coregraph/config.toml"
      ),
    },
  };

  client = new LanguageClient(
    "coregraph",
    "CoreGraph",
    serverOptions,
    clientOptions
  );
  client.start();

  // Foundation instances (IC-1, IC-2, IC-3).
  const tracker = createDaemonHealthTracker();
  const symbolCache = createDocumentSymbolCache();

  const ipcPath = resolveSocketPath();

  // Probe the socket with a short connect — if someone else (typically
  // the `coregraph lsp` process auto-spawned by the LanguageClient)
  // already brought the daemon up, we must not spawn a second one.
  // Two `server start` processes race on the same socket; the loser
  // emits `graceful shutdown complete` and the winner's graph state
  // may not match the queries the extension issues.
  const probeSocket = (): Promise<boolean> =>
    new Promise((resolve) => {
      const sock = require("node:net").createConnection(ipcPath);
      let done = false;
      const settle = (ok: boolean) => {
        if (done) return;
        done = true;
        try { sock.destroy(); } catch { /* ignore */ }
        resolve(ok);
      };
      sock.once("connect", () => settle(true));
      sock.once("error", () => settle(false));
      setTimeout(() => settle(false), 200);
    });

  const ipc = new IpcClient(ipcPath, 30_000, {
    retryOnRefuse: true,
    // Give the LSP subprocess (which auto-spawns the daemon) more time
    // to complete its own spawn-and-bind before we retry. The LSP's
    // cold-start typically takes 200-400ms to cross the Stream::connect
    // boundary; 1500ms leaves margin for slow disks / rosetta.
    spawnDelayMs: 1500,
    spawnFn: async () => {
      tracker.setState("starting");
      // Two probes separated by a wait. The LSP subprocess may be in
      // the middle of `ipc::ensure_running_or_spawn` — its socket won't
      // appear until `bind()` completes. If the first probe fails,
      // wait another 1.5s before declaring we need to spawn ourselves.
      // This eliminates the race where LSP's daemon is already in flight
      // but hasn't yet bound, which was producing 3 concurrent
      // `server start` processes fighting for the same socket inode.
      if (await probeSocket()) return;
      await new Promise((r) => setTimeout(r, 1500));
      if (await probeSocket()) return;
      const proc = spawn(binary, ["server", "start", "--foreground"], {
        detached: true,
        stdio: "ignore",
      });
      proc.unref();
    },
  });
  ipc.onRequestSuccess(() => tracker.setState("online"));
  ipc.onRequestFailed(() => tracker.setState("offline"));

  context.subscriptions.push(tracker, ipc, symbolCache);

  // Restrict watcher to workspace files in supported languages so that
  // settings.json, output-panel buffers, ~/Library/... files do not
  // trigger a reindex on an unrelated project root. The pure helper
  // (util/languageFilter.ts) is unit-tested; here we just pair it
  // with the VSCode-side workspace-folder lookup.
  const isSupportedDoc = (doc: vscode.TextDocument): boolean =>
    isSupportedDocHelper(doc, !!vscode.workspace.getWorkspaceFolder(doc.uri));

  registerDocumentWatcher(context, ipc, { shouldHandle: isSupportedDoc });

  // Project root resolver shared across providers.
  const projectResolver = (doc: vscode.TextDocument): string => {
    const folder = vscode.workspace.getWorkspaceFolder(doc.uri);
    return folder ? folder.uri.fsPath : process.cwd();
  };

  const workspaceRoot = (): string | undefined => {
    const folders = vscode.workspace.workspaceFolders;
    return folders && folders.length > 0 ? folders[0].uri.fsPath : undefined;
  };

  // Providers — signatures accept tracker/symbolCache per providers dev spec.
  // codelensProvider, treeProvider: third arg = symbolCache (IC-3)
  // statusBarProvider: third arg = tracker (IC-1)
  // commitWarning: third arg = tracker (IC-1)
  // Providers — constructor signatures updated by providers dev (IC-1, IC-3).
  const codelensProvider = new CoreGraphCodeLensProvider(ipc, projectResolver, symbolCache);
  const hoverProvider = new CoreGraphHoverProvider(ipc, projectResolver);

  const diagnosticsProvider = new CoreGraphDiagnosticsProvider(ipc, (doc) => {
    if (doc) {
      const folder = vscode.workspace.getWorkspaceFolder(doc.uri);
      if (folder) return folder.uri.fsPath;
    }
    const folders = vscode.workspace.workspaceFolders;
    if (folders && folders.length > 0) return folders[0].uri.fsPath;
    return process.cwd();
  });

  const statusBarProvider = new CoreGraphStatusBarProvider(ipc, projectResolver, tracker);
  const treeProvider = new CoreGraphTreeProvider(ipc, projectResolver, symbolCache);
  const commitWarning = new CoreGraphCommitWarningProvider(ipc, workspaceRoot, tracker);

  context.subscriptions.push(
    vscode.languages.registerCodeLensProvider(selector, codelensProvider),
    { dispose: () => codelensProvider.clear() },
  );

  context.subscriptions.push(
    vscode.languages.registerHoverProvider(selector, hoverProvider),
    { dispose: () => hoverProvider.clear() },
  );

  context.subscriptions.push(diagnosticsProvider);

  statusBarProvider.attach(context);

  const treeView = vscode.window.createTreeView("coregraph.explorer", {
    treeDataProvider: treeProvider,
    showCollapseAll: true,
  });
  context.subscriptions.push(treeView, treeProvider);

  // hasSymbolFile context key — drives viewsWelcome visibility (IC-5).
  function updateHasSymbolFile(): void {
    const editor = vscode.window.activeTextEditor;
    const has = !!(
      editor &&
      editor.document.uri.scheme === "file" &&
      SUPPORTED_LANGUAGES.has(editor.document.languageId)
    );
    void vscode.commands.executeCommand("setContext", "coregraph.hasSymbolFile", has);
  }

  updateHasSymbolFile();

  // Active editor change — 150 ms debounce to avoid rapid UI thrash (IC-5, R-C6e).
  let activeEditorDebounce: NodeJS.Timeout | undefined;
  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor(() => {
      clearTimeout(activeEditorDebounce);
      activeEditorDebounce = setTimeout(() => {
        updateHasSymbolFile();
        treeProvider.refresh();
      }, 150);
    }),
    { dispose: () => clearTimeout(activeEditorDebounce) },
  );

  context.subscriptions.push(
    vscode.commands.registerCommand(
      "coregraph.tree.revealSymbol",
      async (file: string, line: number, col: number) => {
        const uri = vscode.Uri.file(file);
        const doc = await vscode.workspace.openTextDocument(uri);
        const editor = await vscode.window.showTextDocument(doc);
        const pos = new vscode.Position(line, col);
        editor.selection = new vscode.Selection(pos, pos);
        editor.revealRange(new vscode.Range(pos, pos));
      },
    ),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.statusBarClick", () => {
      void vscode.commands.executeCommand("coregraph.explorer.focus");
    }),
  );

  const fileDecorationProvider = new CoreGraphFileDecorationProvider(
    ipc,
    workspaceRoot,
  );
  context.subscriptions.push(
    vscode.window.registerFileDecorationProvider(fileDecorationProvider),
    fileDecorationProvider,
  );
  void fileDecorationProvider.refresh();

  const gitHeadWatcher = vscode.workspace.createFileSystemWatcher("**/.git/HEAD");
  context.subscriptions.push(
    gitHeadWatcher,
    gitHeadWatcher.onDidChange(() => {
      void fileDecorationProvider.refresh();
      void commitWarning.refresh();
    }),
    gitHeadWatcher.onDidCreate(() => {
      void fileDecorationProvider.refresh();
      void commitWarning.refresh();
    }),
  );

  const gutterProvider = new CoreGraphGutterDecorationProvider(
    ipc,
    workspaceRoot,
  );
  gutterProvider.attach(context);
  void gutterProvider.refresh();

  commitWarning.attach(context);
  void commitWarning.refresh();

  // Save handler — single sequenced flow (IC-8, R-C6f).
  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument(async (doc) => {
      // Filter: only act on supported-language files inside a workspace.
      // Otherwise settings.json / Output panel / log files would trigger
      // reindex against unrelated roots and hang on massive directories.
      if (!isSupportedDoc(doc)) return;
      const project = projectResolver(doc);
      try {
        await ipc.reindex(doc.uri.fsPath, "full", project, 10_000);
      } catch {
        // Timeout or failure — tracker transitions to offline via onRequestFailed.
      }
      codelensProvider.invalidate(doc.uri);
      hoverProvider.invalidate(doc.uri);
      void diagnosticsProvider.refresh(doc);
      statusBarProvider.invalidateAll();
      treeProvider.refresh();
      void fileDecorationProvider.refresh();
      void gutterProvider.refresh();
      void commitWarning.refresh();
      vscode.window.setStatusBarMessage("$(check) CoreGraph: graph refreshed", 3000);
    }),
  );

  void diagnosticsProvider.refresh();

  // Diff Impact WebviewPanel.
  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.showDiffImpact", () => {
      DiffImpactPanel.createOrShow(
        ipc,
        workspaceRoot,
        context.extensionUri,
      );
    }),
  );

  // Review Preview WebviewPanel.
  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.showReviewPreview", () => {
      ReviewPreviewPanel.createOrShow(ipc, workspaceRoot);
    }),
  );

  // Daemon status command — info message + Output channel log (R-C6h).
  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.daemonStatus", async () => {
      const project = workspaceRoot() ?? "(none)";
      try {
        const h = await ipc.health();
        outputChannel.appendLine(
          `[${new Date().toISOString()}] daemon v${h.version}, ${h.symbols} symbols, project ${project}`,
        );
        vscode.window.showInformationMessage(
          `CoreGraph daemon v${h.version} — ${h.symbols} symbols indexed · project: ${project}`,
        );
      } catch (e) {
        outputChannel.appendLine(
          `[${new Date().toISOString()}] daemon error: ${e instanceof Error ? e.message : String(e)}`,
        );
        vscode.window.showWarningMessage(
          `CoreGraph daemon not reachable: ${e instanceof Error ? e.message : String(e)}`,
        );
      }
    }),
  );

  // Daemon restart — modal confirmation + progress notification (R-C6g, R2-M5).
  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.daemonRestart", async () => {
      const answer = await vscode.window.showWarningMessage(
        "Restart the CoreGraph daemon? All providers will refresh.",
        { modal: true },
        "Restart",
      );
      if (answer !== "Restart") return;

      codelensProvider.clear();
      statusBarProvider.invalidateAll();
      treeProvider.refresh();

      await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: "CoreGraph: Restarting daemon\u2026" },
        async () => {
          try {
            await new Promise<void>((resolve, reject) => {
              const proc = spawn(binary, ["server", "stop"], { stdio: "ignore" });
              proc.on("close", (code) =>
                code === 0 || code === 1 ? resolve() : reject(new Error(`exit ${code}`)),
              );
              proc.on("error", reject);
            });
          } catch { /* daemon was not running — that is fine */ }
          // Poll health up to 30 s: cold-start indexing of a large
          // workspace can legitimately exceed 5 s, so a single bounded
          // request would time out before the graph is ready. Retry
          // every 500 ms with a short per-probe timeout.
          const deadline = Date.now() + 30_000;
          let lastErr: unknown;
          while (Date.now() < deadline) {
            try {
              await ipc.health(2_000);
              void diagnosticsProvider.refresh();
              void fileDecorationProvider.refresh();
              void gutterProvider.refresh();
              void commitWarning.refresh();
              treeProvider.refresh();
              vscode.window.showInformationMessage("CoreGraph daemon restarted.");
              return;
            } catch (e) {
              lastErr = e;
              await new Promise((r) => setTimeout(r, 500));
            }
          }
          vscode.window.showWarningMessage(
            `CoreGraph daemon restart failed: ${lastErr instanceof Error ? lastErr.message : String(lastErr)}`,
          );
        },
      );
    }),
  );

  // Daemon stop — modal confirmation (R-C6g).
  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.daemonStop", async () => {
      const answer = await vscode.window.showWarningMessage(
        "Stop the CoreGraph daemon? Features will be unavailable until next save.",
        { modal: true },
        "Stop",
      );
      if (answer !== "Stop") return;
      try {
        await new Promise<void>((resolve, reject) => {
          const proc = spawn(binary, ["server", "stop"], { stdio: "ignore" });
          proc.on("close", (code) =>
            code === 0 || code === 1 ? resolve() : reject(new Error(`exit ${code}`)),
          );
          proc.on("error", reject);
        });
        vscode.window.showInformationMessage("CoreGraph daemon stopped.");
      } catch (e) {
        vscode.window.showWarningMessage(
          `CoreGraph: stop failed — ${e instanceof Error ? e.message : String(e)}`,
        );
      }
    }),
  );

  // Show Logs command (R-C6i).
  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.showLogs", () => outputChannel.show()),
  );

  // Open Walkthrough command (R-C6j).
  // ID format: "{publisher}.{extensionName}#{walkthroughId}"
  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.openWalkthrough", () => {
      void vscode.commands.executeCommand(
        "workbench.action.openWalkthrough",
        "coregraph.coregraph-vscode#coregraph.getStarted",
      );
    }),
  );

  // Internal no-op for CodeLens items that carry data but no action.
  context.subscriptions.push(
    vscode.commands.registerCommand("coregraph.codelensNoop", () => {}),
  );

  // Navigate to edge target symbol via workspace symbol provider (R-C6k).
  async function navigateToSymbol(name: string): Promise<void> {
    const symbols = await vscode.commands.executeCommand<vscode.SymbolInformation[]>(
      "vscode.executeWorkspaceSymbolProvider",
      name,
    );
    if (!symbols || symbols.length === 0) {
      vscode.window.showInformationMessage(`CoreGraph: symbol '${name}' not found in workspace`);
      return;
    }
    let target: vscode.SymbolInformation;
    if (symbols.length === 1) {
      target = symbols[0];
    } else {
      const pick = await vscode.window.showQuickPick(
        symbols.map((s) => ({ label: s.name, description: s.location.uri.fsPath, symbol: s })),
        { placeHolder: `Multiple matches for '${name}' — choose one` },
      );
      if (!pick) return;
      target = pick.symbol;
    }
    const doc = await vscode.workspace.openTextDocument(target.location.uri);
    const editor = await vscode.window.showTextDocument(doc);
    editor.revealRange(
      target.location.range,
      vscode.TextEditorRevealType.InCenterIfOutsideViewport,
    );
    editor.selection = new vscode.Selection(
      target.location.range.start,
      target.location.range.start,
    );
  }

  context.subscriptions.push(
    vscode.commands.registerCommand(
      "coregraph.tree.revealEdgeSymbol",
      async (name: string) => navigateToSymbol(name),
    ),
    vscode.commands.registerCommand(
      "coregraph.showImpact",
      async (_uri: vscode.Uri, name: string) => navigateToSymbol(name),
    ),
  );

  // firstRun nudge — shown once, permanently dismissed after any choice (R-C6c).
  // "Later" → globalState persisted → never reshown.
  const shown = context.globalState.get<boolean>("coregraph.firstRunShown", false);
  if (!shown) {
    void (async () => {
      const choice = await vscode.window.showInformationMessage(
        "CoreGraph is active. Enable commit-time impact warnings?",
        "Enable",
        "Later",
        "Learn more",
      );
      if (choice === "Enable") {
        await vscode.workspace
          .getConfiguration("coregraph")
          .update("warnOnCommit.enabled", true, vscode.ConfigurationTarget.Global);
      } else if (choice === "Learn more") {
        void vscode.commands.executeCommand("coregraph.openWalkthrough");
      }
      // Any choice (including dismiss) marks as shown permanently.
      await context.globalState.update("coregraph.firstRunShown", true);
    })();
  }
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
