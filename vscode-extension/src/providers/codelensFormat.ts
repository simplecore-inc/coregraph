import type { ImpactEntry } from "../ipc/types";

/** Numeric SymbolKind values per the VS Code API. Stable since v1.0;
 * documented at https://code.visualstudio.com/api/references/vscode-api#SymbolKind.
 *
 * Re-declared as plain numbers here so this module has no runtime
 * dependency on the `vscode` namespace — it can be unit-tested
 * outside the extension host. */
export const SYMBOL_KIND = {
  File: 0,
  Module: 1,
  Namespace: 2,
  Package: 3,
  Class: 4,
  Method: 5,
  Property: 6,
  Field: 7,
  Constructor: 8,
  Enum: 9,
  Interface: 10,
  Function: 11,
  Variable: 12,
  Constant: 13,
  EnumMember: 21,
  Struct: 22,
} as const;

/** Format the CodeLens title from an impact entry.
 *
 * `nodes` is the count of symbols reachable from this declaration via
 * impact-bearing edges — both callers (incoming) and callees (outgoing)
 * combined. The seed itself is NOT included in `nodes` (the Rust walk
 * tracks visited IDs and only appends *neighbors* to `reachable`).
 *
 * We use "reach N" rather than "N callers" to be honest about what
 * the graph traversal actually computes. */
export function formatTitle(entry: ImpactEntry | null): string {
  if (entry === null) {
    return "CoreGraph: symbol not in graph";
  }
  if (entry.nodes === 0) {
    return "Orphan — entry point or dead code";
  }
  return `reach ${entry.nodes} · impact ${entry.confidence_weighted.toFixed(1)}`;
}

/** Build a tooltip explaining the CodeLens metric values.
 *
 * Surfaces in the CodeLens hover popup so users understand what
 * "reach" and "impact" mean without consulting documentation. */
export function formatCodeLensTooltip(entry: ImpactEntry | null, threshold?: number): string {
  if (entry === null) {
    return (
      "CoreGraph: symbol not found in the dependency graph.\n" +
      "Trigger a reindex: save the file, or run CoreGraph: Restart Daemon from the Command Palette."
    );
  }
  if (entry.nodes === 0) {
    return (
      "Orphan: no other symbols call or reference this one.\n" +
      "It may be dead code or an entry point not yet indexed."
    );
  }
  const lines = [
    `reach ${entry.nodes}: symbols reachable from here via call/use edges`,
    `impact ${entry.confidence_weighted.toFixed(1)}: sum of confidence weights across reachable paths — higher means more central to the graph`,
    "High impact = refactoring this symbol affects many others — test/review carefully.",
  ];
  if (threshold != null) {
    lines.push(`(warn threshold: ${threshold} — change in settings)`);
  }
  return lines.join("\n");
}

/** Symbol kinds that deserve a CodeLens badge. Classes, functions,
 * methods, interfaces — things with inbound callers or outbound
 * impact. Variables, constants, fields, enum-variants, properties are
 * too noisy at CodeLens density. */
export function isDeclKind(kind: number): boolean {
  return (
    kind === SYMBOL_KIND.Function ||
    kind === SYMBOL_KIND.Method ||
    kind === SYMBOL_KIND.Class ||
    kind === SYMBOL_KIND.Interface ||
    kind === SYMBOL_KIND.Struct ||
    kind === SYMBOL_KIND.Enum ||
    kind === SYMBOL_KIND.Constructor
  );
}

/** Structural type matching the relevant subset of vscode.DocumentSymbol.
 * Lets us walk the tree without depending on the vscode namespace. */
export interface SymbolLike<R = unknown> {
  name: string;
  kind: number;
  selectionRange: R;
  children?: SymbolLike<R>[];
}

/** Flatten a (possibly nested) document-symbol tree into a flat list of
 * declarations whose kind passes `isDeclKind`. Preserves each
 * declaration's `selectionRange` opaquely. */
export function flattenAndFilterDecls<R>(
  symbols: SymbolLike<R>[],
): Array<{ name: string; range: R }> {
  const out: Array<{ name: string; range: R }> = [];
  const stack: SymbolLike<R>[] = [...symbols];
  while (stack.length > 0) {
    const s = stack.pop()!;
    if (isDeclKind(s.kind)) {
      out.push({ name: s.name, range: s.selectionRange });
    }
    if (s.children && s.children.length > 0) {
      stack.push(...s.children);
    }
  }
  return out;
}
