import * as vscode from "vscode";
import { IpcClient, CoreGraphError } from "../ipc/client";
import type {
  OrphansResponse,
  InconsistenciesResponse,
} from "../ipc/types";
import {
  buildDiagnostics,
  groupByFile,
  type RawDiagnostic,
} from "./diagnosticsFormat";

/** Phase 1.3 Diagnostics provider.
 *
 * Pulls workspace-wide orphans and inconsistencies after each save and
 * publishes them to the VSCode Problems panel via a dedicated
 * `DiagnosticCollection`. The refresh is debounced implicitly by the
 * documentWatcher's reindex — we only refresh on save, not on
 * keystroke, so the panel doesn't flicker during typing.
 *
 * Per spec §5.3: warnings for inconsistencies, information for
 * orphans. The collection name "coregraph" lets users filter Problems
 * panel by source. */
export class CoreGraphDiagnosticsProvider {
  private readonly collection: vscode.DiagnosticCollection;

  constructor(
    private readonly ipc: IpcClient,
    private readonly projectRootResolver: (
      doc?: vscode.TextDocument,
    ) => string,
  ) {
    this.collection = vscode.languages.createDiagnosticCollection("coregraph");
  }

  /** Re-fetch orphans + inconsistencies for the project that owns
   * `triggeringDoc`. Replaces the entire collection's contents; old
   * diagnostics for files no longer surfaced are cleared. */
  public async refresh(triggeringDoc?: vscode.TextDocument): Promise<void> {
    const project = this.projectRootResolver(triggeringDoc);
    // Honour the user's exclude-tests preference. Default is true so
    // Problems panel stays signal-dense on large workspaces; users can
    // disable to see fixture-level inconsistencies during debugging.
    const excludeTests = vscode.workspace
      .getConfiguration("coregraph")
      .get<boolean>("diagnostics.excludeTests", true);
    const params = {
      output_format: "json",
      exclude_tests: excludeTests,
    };

    let orphans: OrphansResponse;
    let inconsistencies: InconsistenciesResponse;
    try {
      const [o, i] = await Promise.all([
        this.ipc.request<OrphansResponse>("orphans", params, project),
        this.ipc.request<InconsistenciesResponse>(
          "inconsistencies",
          params,
          project,
        ),
      ]);
      orphans = o;
      inconsistencies = i;
    } catch (e) {
      const msg = e instanceof CoreGraphError ? e.message : String(e);
      console.debug(`CoreGraph: diagnostics refresh failed: ${msg}`);
      return;
    }

    const raw = buildDiagnostics(orphans, inconsistencies);
    const grouped = groupByFile(raw);

    // Build a fresh map of vscode entries; .set with an array
    // *replaces* per-file diagnostics. To clear files no longer
    // present, we capture the old set first then explicitly clear
    // those that don't appear in the new set.
    const previousFiles = new Set<string>();
    this.collection.forEach((uri) => previousFiles.add(uri.fsPath));

    const updates: Array<[vscode.Uri, vscode.Diagnostic[]]> = [];
    for (const [file, diags] of grouped) {
      const uri = vscode.Uri.file(file);
      updates.push([uri, diags.map(toVscodeDiagnostic)]);
      previousFiles.delete(file);
    }

    // Apply new diagnostics.
    for (const [uri, diags] of updates) {
      this.collection.set(uri, diags);
    }
    // Clear files that no longer have diagnostics.
    for (const file of previousFiles) {
      this.collection.set(vscode.Uri.file(file), []);
    }
  }

  /** Drop all diagnostics for `uri` (e.g. on file close). */
  public clearFile(uri: vscode.Uri): void {
    this.collection.delete(uri);
  }

  /** Drop the entire collection. Called on disposal. */
  public dispose(): void {
    this.collection.dispose();
  }
}

function toVscodeDiagnostic(d: RawDiagnostic): vscode.Diagnostic {
  // Single-character range at the symbol's line; Problems panel only
  // uses the start position to anchor the entry. Phase 1.4+ may
  // upgrade to span_end-based highlight if the IPC starts returning
  // span_end alongside line.
  const range = new vscode.Range(d.line, 0, d.line, Number.MAX_SAFE_INTEGER);
  const severity =
    d.severity === "warning"
      ? vscode.DiagnosticSeverity.Warning
      : vscode.DiagnosticSeverity.Information;
  const diag = new vscode.Diagnostic(range, d.message, severity);
  diag.source = d.source;
  diag.code = d.code;
  return diag;
}
