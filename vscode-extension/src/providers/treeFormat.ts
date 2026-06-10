import type { InspectResult, EdgeInfo } from "../ipc/types";

/** All label/description formatting lives here. Pure functions only —
 * no vscode dep, easy to unit-test. */

export type TreeNode =
  | { kind: "file"; file: string }
  | {
      kind: "symbol";
      file: string;
      name: string;
      symbolKind: string;
      line: number;
      col: number;
      // Populated after first inspect; absent on initial render.
      edges?: number;
      staleCount?: number;
    }
  | {
      kind: "direction";
      file: string;
      symbolName: string;
      direction: "incoming" | "outgoing";
      count: number;
    }
  | {
      kind: "edge";
      otherName: string; // "from" for incoming, "to" for outgoing
      edgeKind: string;
      confidence: number;
      stale: number;
    };

/** Format a symbol node's main label: just the name. */
export function symbolLabel(name: string): string {
  return name;
}

/** Format the description shown after the label. Includes [Kind] and
 * (optionally) edges + stale once we've resolved the symbol via inspect. */
export function symbolDescription(
  node: Extract<TreeNode, { kind: "symbol" }>,
): string {
  const parts: string[] = [`[${node.symbolKind}]`];
  if (node.edges !== undefined) parts.push(`edges ${node.edges}`);
  if (node.staleCount !== undefined && node.staleCount > 0) {
    parts.push(`⚠ ${node.staleCount} stale`);
  }
  return parts.join(" · ");
}

export function directionLabel(
  node: Extract<TreeNode, { kind: "direction" }>,
): string {
  const arrow = node.direction === "incoming" ? "Incoming" : "Outgoing";
  return `${arrow} (${node.count})`;
}

export function edgeLabel(node: Extract<TreeNode, { kind: "edge" }>): string {
  return node.otherName;
}

/** Description: confidence % + kind + stale suffix. */
export function edgeDescription(
  node: Extract<TreeNode, { kind: "edge" }>,
): string {
  const conf = Math.round(node.confidence * 100) + "%";
  const staleSuffix = node.stale > 0 ? ` · stale ${node.stale}` : "";
  return `${conf} · ${node.edgeKind}${staleSuffix}`;
}

/** Build symbol children from an inspect response: two direction nodes
 * with the resolved counts. Returns empty when no edges in either
 * direction (the symbol is then just a leaf in the tree). */
export function symbolToDirectionNodes(
  file: string,
  symbolName: string,
  inspect: InspectResult,
): Array<Extract<TreeNode, { kind: "direction" }>> {
  const out: Array<Extract<TreeNode, { kind: "direction" }>> = [];
  if (inspect.edges_in.length > 0) {
    out.push({
      kind: "direction",
      file,
      symbolName,
      direction: "incoming",
      count: inspect.edges_in.length,
    });
  }
  if (inspect.edges_out.length > 0) {
    out.push({
      kind: "direction",
      file,
      symbolName,
      direction: "outgoing",
      count: inspect.edges_out.length,
    });
  }
  return out;
}

/** Convert a list of EdgeInfo into edge tree nodes, sorted by
 * confidence descending. The `endpointKey` selects which side of the
 * edge is shown ("from" for incoming, "to" for outgoing). */
export function edgesToNodes(
  edges: EdgeInfo[],
  endpointKey: "from" | "to",
): Array<Extract<TreeNode, { kind: "edge" }>> {
  const sorted = [...edges].sort((a, b) => {
    if (b.confidence !== a.confidence) return b.confidence - a.confidence;
    return a.kind.localeCompare(b.kind);
  });
  return sorted.map((e) => ({
    kind: "edge" as const,
    otherName: e[endpointKey] ?? "<unknown>",
    edgeKind: e.kind,
    confidence: e.confidence,
    stale: e.stale,
  }));
}

/** Build a Markdown tooltip for an edge node.
 *
 * Explains confidence (inference reliability) and stale evidence so users
 * know what the numbers mean without reading the documentation. */
export function edgeTooltipMarkdown(
  node: Extract<TreeNode, { kind: "edge" }>,
): string {
  const confPct = (node.confidence * 100).toFixed(0);
  const lines: string[] = [
    `**${node.otherName}**`,
    "",
    `Edge type: **${node.edgeKind}**`,
    `Confidence: **${confPct}%** — how reliably this edge was inferred`,
    "_(Higher values come from facts the extractor observed directly or resolved by name; lower values are inferred from code structure and may be approximate.)_",
  ];
  if (node.stale > 0) {
    lines.push(
      "",
      `⚠ **Stale:** last confirmed ${node.stale} reindex(es) ago — may no longer exist`,
    );
  }
  lines.push("", "_Use the Go-to-symbol link below to navigate._");
  return lines.join("\n");
}

/** Helper: extract symbol-level edge count + stale-count summary from an
 * inspect result. edges = total edges in + out. */
export function summarizeSymbol(inspect: InspectResult): {
  edges: number;
  staleCount: number;
} {
  const edges = inspect.edges_in.length + inspect.edges_out.length;
  let staleCount = 0;
  for (const e of inspect.edges_in) if (e.stale > 0) staleCount += 1;
  for (const e of inspect.edges_out) if (e.stale > 0) staleCount += 1;
  return { edges, staleCount };
}
