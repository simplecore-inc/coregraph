import type { DiffResult, DiffFileImpact } from "../ipc/types";

export type ImpactTier = "low" | "medium" | "high";

/** Classify a confidence-weighted impact score into a severity tier
 * matching spec §6.1: <5 green, 5-20 yellow, >20 red. */
export function impactTier(confidence_weighted: number): ImpactTier {
  if (confidence_weighted > 20) return "high";
  if (confidence_weighted >= 5) return "medium";
  return "low";
}

/** VSCode theme color id for the impact tier. These MUST exist in
 * package.json's contributes.colors so the theme respects them. */
export function themeColorId(tier: ImpactTier): string {
  switch (tier) {
    case "high":   return "coregraph.impact.high";
    case "medium": return "coregraph.impact.medium";
    case "low":    return "coregraph.impact.low";
  }
}

/** Single-char (or 2-char) badge for the file decoration. VSCode
 * constrains `badge` to 2 chars — we use a single glyph per tier. */
export function badgeChar(tier: ImpactTier): string {
  switch (tier) {
    case "high":   return "‼";
    case "medium": return "!";
    case "low":    return "•";
  }
}

/** Multi-line tooltip summarizing per-file impact. Top 5 affected
 * symbols come from the IPC response already (2.0 caps at 5). */
export function formatTooltip(f: DiffFileImpact, threshold?: number): string {
  const lines: string[] = [];
  lines.push(`Impact: ${f.confidence_weighted.toFixed(1)}`);
  lines.push(`reach: ${f.reachable_count} symbols`);
  if (f.seed_symbols.length > 0) {
    lines.push(`Seeds: ${f.seed_symbols.join(", ")}`);
  }
  if (f.top_affected.length > 0) {
    lines.push("");
    lines.push("Top affected:");
    for (const a of f.top_affected) {
      lines.push(`  • ${a.name} (${Math.round(a.confidence * 100) + "%"})`);
    }
  }
  if (threshold !== undefined) {
    lines.push(`(warn threshold: ${threshold} — commit above this level triggers warning)`);
  }
  return lines.join("\n");
}

/** Convert a DiffResult into a map keyed by file path for O(1)
 * lookup inside provideFileDecoration. Files in the diff but with
 * zero seeds still get a "low" tier badge so the SCM list distinguishes
 * "tracked by CoreGraph" from "not CoreGraph at all". */
export function toDecorationMap(diff: DiffResult): Map<string, DiffFileImpact> {
  const out = new Map<string, DiffFileImpact>();
  for (const f of diff.changed_files) {
    out.set(f.file, f);
  }
  return out;
}
