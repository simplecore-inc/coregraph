import type { InspectResult, EdgeInfo } from "../ipc/types";

/** Maximum neighbors to list per direction in the Hover body. The
 * Hover popup is small and a 96-caller list would scroll off-screen. */
const TOP_NEIGHBORS = 5;

/** Render an `inspect` IPC response as Markdown for VSCode's Hover popup.
 *
 * Pure function — no `vscode` dependency, easy to unit-test.
 *
 * Structure:
 *   1. Symbol header (name + kind), or null-symbol notice.
 *   2. File:line origin link (plain text — VSCode doesn't auto-link
 *      these in hover Markdown without `vscode.Uri.file(...).fsPath`).
 *   3. Incoming neighbors summary + top N by confidence.
 *   4. Outgoing neighbors summary + top N by confidence.
 *   5. Staleness signal (only if any edge has `stale > 0`).
 *   6. Freshness footer (last rebuild age).
 */
export function formatHoverMarkdown(result: InspectResult): string {
  if (result.symbol === null) {
    return [
      "**CoreGraph**",
      "",
      "_No symbol resolved at this position._",
      "",
      `Last rebuild: ${formatAge(result.freshness.last_rebuild_at_ms)} ago.`,
    ].join("\n");
  }

  const sym = result.symbol;
  const lines: string[] = [];

  lines.push(`**\`${sym.name}\`** — _${sym.kind}_`);
  lines.push("");
  lines.push(`\`${sym.file}:${sym.line}\``);
  lines.push("");
  lines.push("---");
  lines.push("");

  // Incoming.
  lines.push(`**Incoming** (${result.edges_in.length})`);
  if (result.edges_in.length === 0) {
    lines.push("_No callers known._");
  } else {
    for (const line of formatNeighborList(result.edges_in, "from")) {
      lines.push(line);
    }
  }
  lines.push("");

  // Outgoing.
  lines.push(`**Outgoing** (${result.edges_out.length})`);
  if (result.edges_out.length === 0) {
    lines.push("_No callees known._");
  } else {
    for (const line of formatNeighborList(result.edges_out, "to")) {
      lines.push(line);
    }
  }
  lines.push("");

  // Stale signal.
  const staleCount = countStaleEdges(result.edges_in, result.edges_out);
  if (staleCount > 0) {
    lines.push(`⚠ **Stale evidence:** ${staleCount} edge(s) with stale > 0`);
    lines.push("");
  }

  // Origin distribution — useful to see whether name-resolved or syntactic.
  const originSummary = summarizeOrigin([...result.edges_in, ...result.edges_out]);
  if (originSummary.length > 0) {
    lines.push(`**Sources:** ${originSummary}`);
    lines.push("");
  }

  lines.push(`_Last rebuild: ${formatAge(result.freshness.last_rebuild_at_ms)} ago._`);

  return lines.join("\n");
}

function formatNeighborList(
  edges: EdgeInfo[],
  endpointKey: "from" | "to",
): string[] {
  // Sort by confidence descending, take top N. Stable ordering for
  // the secondary key (kind) so the same input always produces the
  // same Markdown.
  const sorted = [...edges].sort((a, b) => {
    if (b.confidence !== a.confidence) return b.confidence - a.confidence;
    return a.kind.localeCompare(b.kind);
  });
  const top = sorted.slice(0, TOP_NEIGHBORS);
  const rest = sorted.length - top.length;

  const out = top.map((e) => {
    const name = e[endpointKey] ?? "<unknown>";
    const conf = Math.round(e.confidence * 100) + "%";
    const staleSuffix = e.stale > 0 ? ` (stale ${e.stale})` : "";
    return `- \`${name}\` — ${conf} · ${e.kind}${staleSuffix}`;
  });
  if (rest > 0) {
    out.push(`- _… ${rest} more_`);
  }
  return out;
}

function countStaleEdges(...groups: EdgeInfo[][]): number {
  let n = 0;
  for (const g of groups) {
    for (const e of g) {
      if (e.stale > 0) n += 1;
    }
  }
  return n;
}

function summarizeOrigin(edges: EdgeInfo[]): string {
  if (edges.length === 0) return "";
  const counts = new Map<string, number>();
  for (const e of edges) {
    counts.set(e.origin, (counts.get(e.origin) ?? 0) + 1);
  }
  const sorted = [...counts.entries()].sort((a, b) => b[1] - a[1]);
  return sorted.map(([origin, n]) => `${origin} × ${n}`).join(", ");
}

/** Convert a millisecond duration into a compact human-readable string.
 *
 * Examples: `12 ms`, `1.4 s`, `42 s`, `3 min`, `1.2 h`.
 *
 * `inspect` returns `freshness.last_rebuild_at_ms` as the elapsed-since
 * (`Instant::elapsed().as_millis()`), so we display it as an age. */
export function formatAge(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)} s`;
  const m = s / 60;
  if (m < 60) return `${Math.round(m)} min`;
  const h = m / 60;
  return `${h.toFixed(1)} h`;
}
