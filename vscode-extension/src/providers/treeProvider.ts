import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type { InspectResult } from "../ipc/types";
import {
  type TreeNode,
  symbolLabel,
  symbolDescription,
  directionLabel,
  edgeLabel,
  edgeDescription,
  edgeTooltipMarkdown,
  symbolToDirectionNodes,
  edgesToNodes,
  summarizeSymbol,
} from "./treeFormat";
import {
  type SymbolLike,
  flattenAndFilterDecls,
} from "./codelensFormat";
import type { DocumentSymbolCache } from "../util/documentSymbolCache";
import { LRUMap } from "../util/lruMap";

const INSPECT_CACHE_TTL_MS = 5000;

const SUPPORTED_LANGUAGES = [
  "rust",
  "java",
  "typescript",
  "typescriptreact",
  "javascript",
  "javascriptreact",
  "python",
  "go",
  "kotlin",
];

interface InspectCacheEntry {
  at: number;
  result: InspectResult;
}

export class CoreGraphTreeProvider
  implements vscode.TreeDataProvider<TreeNode>, vscode.Disposable
{
  private readonly _onDidChange = new vscode.EventEmitter<
    TreeNode | undefined
  >();
  public readonly onDidChangeTreeData = this._onDidChange.event;

  // Keyed by `${file}|${line}|${col}`.
  private readonly inspectCache = new LRUMap<string, InspectCacheEntry>(256);

  // Keyed by `${file}|${symbolName}` — populated as a side-effect of
  // symbolChildren so that directionChildren can look up inspect results
  // by symbol name alone.
  private readonly inspectCacheBySymbol = new LRUMap<string, InspectCacheEntry>(256);

  constructor(
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: (doc: vscode.TextDocument) => string,
    private readonly symbolCache: DocumentSymbolCache,
  ) {}

  /** Refresh the entire tree. Called on active-editor change or post-reindex. */
  public refresh(): void {
    this.inspectCache.clear();
    this.inspectCacheBySymbol.clear();
    this._onDidChange.fire(undefined);
  }

  public getTreeItem(node: TreeNode): vscode.TreeItem {
    switch (node.kind) {
      case "file": {
        const item = new vscode.TreeItem(
          basename(node.file),
          vscode.TreeItemCollapsibleState.Expanded,
        );
        item.description = node.file;
        item.iconPath = new vscode.ThemeIcon("file-code");
        item.tooltip = node.file;
        return item;
      }
      case "symbol": {
        const item = new vscode.TreeItem(
          symbolLabel(node.name),
          vscode.TreeItemCollapsibleState.Collapsed,
        );
        item.description = symbolDescription(node);
        item.iconPath = symbolIconForKind(node.symbolKind);
        item.command = {
          title: "Reveal symbol",
          command: "coregraph.tree.revealSymbol",
          arguments: [node.file, node.line, node.col],
        };
        return item;
      }
      case "direction": {
        const item = new vscode.TreeItem(
          directionLabel(node),
          vscode.TreeItemCollapsibleState.Collapsed,
        );
        item.iconPath = new vscode.ThemeIcon(
          node.direction === "incoming" ? "arrow-left" : "arrow-right",
        );
        return item;
      }
      case "edge": {
        const item = new vscode.TreeItem(
          edgeLabel(node),
          vscode.TreeItemCollapsibleState.None,
        );
        item.description = edgeDescription(node);
        item.iconPath = new vscode.ThemeIcon("symbol-method");
        // Trusted MarkdownString with an inline command link for navigation.
        // Row-level command is removed; the tooltip link is the entry point.
        const tooltip = new vscode.MarkdownString(edgeTooltipMarkdown(node));
        tooltip.isTrusted = true;
        const encodedArgs = encodeURIComponent(JSON.stringify([node.otherName]));
        tooltip.appendMarkdown(
          `\n\n[Go to symbol](command:coregraph.tree.revealEdgeSymbol?${encodedArgs})`,
        );
        item.tooltip = tooltip;
        item.contextValue = "cg-edge";
        // No item.command — navigation via tooltip link only (R-P2 re-design).
        return item;
      }
    }
  }

  public async getChildren(node?: TreeNode): Promise<TreeNode[]> {
    if (!node) {
      const editor = vscode.window.activeTextEditor;
      if (
        !editor ||
        editor.document.uri.scheme !== "file" ||
        !SUPPORTED_LANGUAGES.includes(editor.document.languageId)
      ) {
        // Return empty so viewsWelcome renders instead of a "no-file" node.
        return [];
      }
      return [{ kind: "file", file: editor.document.uri.fsPath }];
    }

    // Leaf kind — no children.
    if (node.kind === "edge") {
      return [];
    }

    if (node.kind === "file") {
      return this.fileChildren(node.file);
    }

    if (node.kind === "symbol") {
      return this.symbolChildren(node);
    }

    if (node.kind === "direction") {
      return this.directionChildren(node);
    }

    return [];
  }

  private async fileChildren(file: string): Promise<TreeNode[]> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.uri.fsPath !== file) return [];

    let docSymbols: vscode.DocumentSymbol[];
    try {
      docSymbols = await this.symbolCache.get(editor.document);
    } catch (e) {
      console.debug(
        `CoreGraph: tree document-symbol failed: ${
          e instanceof Error ? e.message : String(e)
        }`,
      );
      return [];
    }

    const decls = flattenAndFilterDecls<vscode.Range>(
      docSymbols as SymbolLike<vscode.Range>[],
    );
    return decls.map((d) => ({
      kind: "symbol" as const,
      file,
      name: d.name,
      symbolKind: kindLabelFromSymbols(docSymbols, d.name),
      line: d.range.start.line,
      col: d.range.start.character,
    }));
  }

  private async symbolChildren(
    node: Extract<TreeNode, { kind: "symbol" }>,
  ): Promise<TreeNode[]> {
    const inspect = await this.inspectSymbol(node.file, node.line, node.col);
    if (!inspect) return [];

    const { edges, staleCount } = summarizeSymbol(inspect);
    if (node.edges !== edges || node.staleCount !== staleCount) {
      node.edges = edges;
      node.staleCount = staleCount;
      this._onDidChange.fire(node);
    }

    return symbolToDirectionNodes(node.file, node.name, inspect);
  }

  private directionChildren(
    node: Extract<TreeNode, { kind: "direction" }>,
  ): TreeNode[] {
    const cached = this.inspectCacheBySymbol.get(
      `${node.file}|${node.symbolName}`,
    );
    if (!cached) return [];
    if (Date.now() - cached.at >= INSPECT_CACHE_TTL_MS) {
      this.inspectCacheBySymbol.delete(`${node.file}|${node.symbolName}`);
      // Stale entry — trigger a full tree refresh so the user sees fresh data.
      this._onDidChange.fire(undefined);
      return [];
    }
    if (node.direction === "incoming") {
      return edgesToNodes(cached.result.edges_in, "from");
    }
    return edgesToNodes(cached.result.edges_out, "to");
  }

  private async inspectSymbol(
    file: string,
    line: number,
    col: number,
  ): Promise<InspectResult | undefined> {
    const posKey = `${file}|${line}|${col}`;
    const cached = this.inspectCache.get(posKey);
    if (cached && Date.now() - cached.at < INSPECT_CACHE_TTL_MS) {
      return cached.result;
    }

    const editor = vscode.window.activeTextEditor;
    const project = editor
      ? this.projectRootResolver(editor.document)
      : process.cwd();

    try {
      const result = await this.ipc.inspect(file, line, col, project);
      const entry: InspectCacheEntry = { at: Date.now(), result };
      this.inspectCache.set(posKey, entry);
      if (result.symbol) {
        this.inspectCacheBySymbol.set(`${file}|${result.symbol.name}`, entry);
      }
      return result;
    } catch (e) {
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      console.debug(`CoreGraph: tree inspect failed: ${msg}`);
      return undefined;
    }
  }

  public dispose(): void {
    this.inspectCache.clear();
    this.inspectCacheBySymbol.clear();
    this._onDidChange.dispose();
  }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

function basename(p: string): string {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i >= 0 ? p.slice(i + 1) : p;
}

function symbolIconForKind(kind: string): vscode.ThemeIcon {
  switch (kind.toLowerCase()) {
    case "function":
      return new vscode.ThemeIcon("symbol-function");
    case "method":
    case "constructor":
      return new vscode.ThemeIcon("symbol-method");
    case "class":
    case "struct":
      return new vscode.ThemeIcon("symbol-class");
    case "interface":
      return new vscode.ThemeIcon("symbol-interface");
    case "enum":
      return new vscode.ThemeIcon("symbol-enum");
    default:
      return new vscode.ThemeIcon("symbol-misc");
  }
}

/** Find the kind label for a symbol by name in a docSymbol tree. */
function kindLabelFromSymbols(
  symbols: vscode.DocumentSymbol[],
  name: string,
): string {
  const stack = [...symbols];
  while (stack.length > 0) {
    const s = stack.pop()!;
    if (s.name === name) return vscode.SymbolKind[s.kind] ?? "Symbol";
    if (s.children && s.children.length > 0) stack.push(...s.children);
  }
  return "Symbol";
}
