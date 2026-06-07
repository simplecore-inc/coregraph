import type { InspectResult, ImpactEntry, EdgeInfo } from "../ipc/types";

/** Result of formatting a single status-bar update.
 *
 * `text` is what shows in the status bar (left side, with $(pulse)
 * codicon). `tooltip` is the hover popup with extra context.
 * `accessibleText` is the plain-English screen-reader label (no codicons,
 * no unicode warning glyphs). */
export interface StatusBarRender {
  text: string;
  tooltip: string;
  /** Plain-English label for accessibilityInformation — no codicons, no unicode warning. */
  accessibleText: string;
  /** True when there is something to show. False = hide the item. */
  visible: boolean;
}

/** Render the status bar text from an inspect result and an optional
 * companion impact entry.
 *
 * - When `inspect.symbol` is `null`: hide the item.
 * - When the symbol resolved but no impact entry: show name only.
 * - When both are present: name + impact + stale count.
 *
 * Format mirrors spec §5.4: `$(pulse) name · impact N.N · ⚠ K stale`.
 *
 * The `$(pulse)` codicon prefix is left to the provider — this pure
 * function returns just the message body so unit tests don't have to
 * stub VSCode's icon syntax. */
export function formatStatusBar(
  inspect: InspectResult,
  impact: ImpactEntry | null,
): StatusBarRender {
  if (inspect.symbol === null) {
    return { text: "", tooltip: "", accessibleText: "", visible: false };
  }
  const name = inspect.symbol.name;
  const stale = countStaleEdges(inspect.edges_in, inspect.edges_out);

  const parts: string[] = [name];
  if (impact !== null) {
    parts.push(`impact ${impact.confidence_weighted.toFixed(1)}`);
  }
  if (stale > 0) {
    parts.push(`⚠ ${stale} stale`);
  }

  const text = parts.join(" · ");
  const tooltip = formatTooltip(inspect, impact, stale);
  const accessibleText = buildAccessibleText(name, impact, stale);
  return { text, tooltip, accessibleText, visible: true };
}

function buildAccessibleText(
  name: string,
  impact: ImpactEntry | null,
  stale: number,
): string {
  let result = name;
  if (impact != null) {
    result += `, impact ${impact.confidence_weighted.toFixed(1)}`;
  }
  if (stale > 0) {
    result += `, ${stale} stale edge${stale === 1 ? "" : "s"}`;
  }
  return result;
}

function formatTooltip(
  inspect: InspectResult,
  impact: ImpactEntry | null,
  stale: number,
): string {
  // Bullet-style multi-line tooltip — VSCode renders newlines.
  const lines: string[] = [];
  const sym = inspect.symbol!;
  lines.push(`${sym.name}  [${sym.kind}]`);
  lines.push(`${sym.file}:${sym.line}`);
  lines.push("");
  lines.push(`In: ${inspect.edges_in.length}  ·  Out: ${inspect.edges_out.length}`);
  if (impact !== null) {
    lines.push(
      `reach ${impact.nodes} · impact ${impact.confidence_weighted.toFixed(1)}`,
    );
  }
  if (stale > 0) {
    lines.push(`${stale} caller/callee edge(s) may be outdated — save a file to trigger reindex`);
  }
  return lines.join("\n");
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
