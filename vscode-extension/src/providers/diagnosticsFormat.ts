import type {
  OrphansResponse,
  InconsistenciesResponse,
  OrphanReport,
  InconsistencyReport,
} from "../ipc/types";

/** Lean diagnostic representation independent of the `vscode`
 * namespace — pure helpers can build these in unit tests, the
 * provider then maps them to `vscode.Diagnostic` instances. */
export interface RawDiagnostic {
  /** Absolute file path (as returned by the daemon JSON). */
  file: string;
  /** 0-based line. */
  line: number;
  /** Severity: `warning` for inconsistencies, `info` for orphans. */
  severity: "warning" | "information";
  /** Message displayed in the Problems panel. */
  message: string;
  /** Stable category code for filtering / suppression. */
  source: "coregraph";
  code: string;
}

/** Group raw diagnostics by file path. The provider feeds each file's
 * list to a `vscode.DiagnosticCollection.set(uri, diagnostics)` call. */
export function groupByFile(
  diagnostics: RawDiagnostic[],
): Map<string, RawDiagnostic[]> {
  const out = new Map<string, RawDiagnostic[]>();
  for (const d of diagnostics) {
    const list = out.get(d.file);
    if (list) {
      list.push(d);
    } else {
      out.set(d.file, [d]);
    }
  }
  return out;
}

/** Render an orphan node into a Problems-panel diagnostic.
 *
 * Severity is `Information` per spec §5.3 — a possibly-dead-code hint,
 * not an error. */
export function orphanToDiagnostic(o: OrphanReport): RawDiagnostic {
  return {
    file: o.file,
    line: o.line,
    severity: "information",
    message: `'${o.name}' has no known callers — possibly dead code (kind: ${o.kind})`,
    source: "coregraph",
    code: "orphan",
  };
}

/** Render an inconsistency report into ONE diagnostic per implicated
 * node — Problems panel surfaces both sides so the user can navigate
 * either. The `code` carries the category for filtering. */
export function inconsistencyToDiagnostics(
  r: InconsistencyReport,
): RawDiagnostic[] {
  const message = formatInconsistencyMessage(r);
  const code = `inconsistency.${r.category}`;
  return [
    {
      file: r.a.file,
      line: r.a.line,
      severity: "warning",
      message,
      source: "coregraph",
      code,
    },
    {
      file: r.b.file,
      line: r.b.line,
      severity: "warning",
      message,
      source: "coregraph",
      code,
    },
  ];
}

function formatInconsistencyMessage(r: InconsistencyReport): string {
  switch (r.category) {
    case "EnumMismatch":
      return `Enum literal '${r.shared_value}' shared by '${r.a.name}' and '${r.b.name}' — same name, different enums`;
    case "ApiPath":
      return `API path mismatch '${r.shared_value}' between '${r.a.file}' and '${r.b.file}'`;
    case "ConfigKey":
      return `Config key '${r.shared_value}' inconsistency: '${r.a.name}' (${r.a.file}) vs '${r.b.name}' (${r.b.file})`;
    default:
      return `Inconsistency (${r.category}): ${r.shared_value} — '${r.a.name}' vs '${r.b.name}'`;
  }
}

/** Combine orphans + inconsistencies responses into a flat raw-diagnostic
 * list. Use `groupByFile` to bucket them by file URI before pushing
 * into the `DiagnosticCollection`. */
export function buildDiagnostics(
  orphans: OrphansResponse,
  inconsistencies: InconsistenciesResponse,
): RawDiagnostic[] {
  const out: RawDiagnostic[] = [];
  for (const o of orphans.orphans) out.push(orphanToDiagnostic(o));
  for (const r of inconsistencies.reports) {
    for (const d of inconsistencyToDiagnostics(r)) out.push(d);
  }
  return out;
}
