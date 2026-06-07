// `import type` is erased by the TypeScript compiler — at runtime this
// module has NO dependency on the `vscode` package, which is only
// available inside the extension host. The default executor below
// loads vscode lazily so unit tests (node + ts-node) can inject a
// fake executor without pulling in the missing module.
import type * as vscode from "vscode";

/**
 * Per-document symbol cache shared by TreeView and CodeLens.
 *
 * Both providers call `vscode.executeDocumentSymbolProvider` on the
 * same document during a single refresh cycle; the cache coalesces
 * those into one round-trip while the document version is stable.
 *
 * Key: `${uri.toString()}@${doc.version}` — version ensures stale
 * entries are naturally bypassed after an edit.
 *
 * TTL is a belt-and-suspenders guard against long-lived buffers where
 * the symbol provider silently went out of sync (extension reloads,
 * language server restarts).
 */
export interface DocumentSymbolCache extends vscode.Disposable {
  get(doc: vscode.TextDocument): Promise<vscode.DocumentSymbol[]>;
}

export type SymbolExecutor = (
  uri: vscode.Uri,
) => Thenable<
  vscode.DocumentSymbol[] | vscode.SymbolInformation[] | undefined
>;

interface Entry {
  at: number;
  promise: Promise<vscode.DocumentSymbol[]>;
}

const DEFAULT_TTL_MS = 2000;

const defaultExecutor: SymbolExecutor = (uri) => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const vscodeApi = require("vscode") as typeof import("vscode");
  return vscodeApi.commands.executeCommand<
    vscode.DocumentSymbol[] | vscode.SymbolInformation[] | undefined
  >("vscode.executeDocumentSymbolProvider", uri);
};

export function createDocumentSymbolCache(
  executor: SymbolExecutor = defaultExecutor,
  ttlMs: number = DEFAULT_TTL_MS,
): DocumentSymbolCache {
  const entries = new Map<string, Entry>();

  return {
    get(doc: vscode.TextDocument): Promise<vscode.DocumentSymbol[]> {
      const key = `${doc.uri.toString()}@${doc.version}`;
      const now = Date.now();
      const hit = entries.get(key);
      if (hit && now - hit.at < ttlMs) {
        return hit.promise;
      }

      const promise = Promise.resolve(executor(doc.uri)).then((raw) =>
        normalize(raw),
      );
      entries.set(key, { at: now, promise });
      return promise;
    },
    dispose(): void {
      entries.clear();
    },
  };
}

/** Normalize the VSCode symbol provider result to DocumentSymbol[].
 *
 * The API can return DocumentSymbol[] (nested — most LSPs), or
 * SymbolInformation[] (flat — older providers). We flatten-map the
 * latter into the former using plain object literals compatible with
 * `SymbolLike<vscode.Range>` (the structural type used by provider
 * helpers). A plain literal is chosen over `new vscode.DocumentSymbol`
 * so this module is unit-testable without a vscode runtime.
 *
 * Consumers read `.name`, `.kind`, `.range`, `.selectionRange`,
 * `.children` — all present on the literal. VSCode never re-ingests
 * these values, so class-identity is not required. */
function normalize(
  raw: vscode.DocumentSymbol[] | vscode.SymbolInformation[] | undefined,
): vscode.DocumentSymbol[] {
  if (!raw || raw.length === 0) return [];
  if ("children" in raw[0]) {
    return raw as vscode.DocumentSymbol[];
  }
  const flat = raw as vscode.SymbolInformation[];
  return flat.map(
    (s): vscode.DocumentSymbol => ({
      name: s.name,
      detail: "",
      kind: s.kind,
      tags: [],
      range: s.location.range,
      selectionRange: s.location.range,
      children: [],
    }),
  );
}
