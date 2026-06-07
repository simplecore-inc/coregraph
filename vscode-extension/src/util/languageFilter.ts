// Pure filter helpers for deciding which VSCode documents the
// extension reacts to. Extracted from extension.ts so the logic is
// unit-testable without stubbing the `vscode` namespace.
//
// Consumer contract (extension.ts):
//   const has = !!vscode.workspace.getWorkspaceFolder(doc.uri);
//   if (isSupportedDoc(doc, has)) { /* reindex / refresh */ }

/** Languages the daemon extracts. Must stay in sync with the LSP
 * `documentSelector` in extension.ts and the `activationEvents` entry
 * in package.json. */
export const SUPPORTED_LANGUAGES: ReadonlySet<string> = new Set([
  "rust",
  "java",
  "typescript",
  "typescriptreact",
  "javascript",
  "javascriptreact",
  "python",
  "go",
  "kotlin",
]);

/** Minimal structural shape of `vscode.TextDocument` we need. Using a
 * structural interface keeps the helper decoupled from the `vscode`
 * namespace (impossible to import under ts-node unit tests). */
export interface DocLike {
  readonly uri: { readonly scheme: string };
  readonly languageId: string;
}

/** `true` when `languageId` is one of the supported extractor targets. */
export function isSupportedLanguage(languageId: string): boolean {
  return SUPPORTED_LANGUAGES.has(languageId);
}

/** Decide whether the extension should act on a document event
 * (reindex, diagnostics refresh, etc.). Callers must first resolve
 * the workspace-folder membership themselves (via
 * `vscode.workspace.getWorkspaceFolder(doc.uri)`) and pass the boolean
 * in. Gating in one place prevents the daemon from being handed files
 * outside the user's workspace root — e.g. the VSCode user settings
 * file, Output panel buffers, or Git scheme diff views — which would
 * otherwise cause the daemon to try to index an unrelated directory
 * and hang. */
export function isSupportedDoc(
  doc: DocLike,
  hasWorkspaceFolder: boolean,
): boolean {
  return (
    doc.uri.scheme === "file" &&
    hasWorkspaceFolder &&
    isSupportedLanguage(doc.languageId)
  );
}
