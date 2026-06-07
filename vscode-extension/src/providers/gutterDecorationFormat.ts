import type { DiffResult } from "../ipc/types";

/** A single gutter decoration hit: one line in one file with associated
 * impact symbols. */
export interface GutterHit {
  file: string;
  /** 0-based line number. */
  line: number;
  symbols: string[];
  confidence_weighted: number;
}

/** Flatten a `DiffResult`'s `per_line_impact` across all files into a flat
 * list of gutter hits. Files that carry no `per_line_impact` (non-daemon
 * fallback path) are skipped silently. */
export function toGutterHits(diff: DiffResult): GutterHit[] {
  const out: GutterHit[] = [];
  for (const f of diff.changed_files) {
    if (!f.per_line_impact) continue;
    for (const [lineStr, info] of Object.entries(f.per_line_impact)) {
      const line = parseInt(lineStr, 10);
      if (Number.isNaN(line)) continue;
      out.push({
        file: f.file,
        line,
        symbols: info.symbols,
        confidence_weighted: info.confidence_weighted,
      });
    }
  }
  return out;
}

/** Compose a Markdown hover string for a gutter hit.
 * Rendered by VSCode as a `MarkdownString` on the decoration. */
export function formatGutterTooltip(hit: GutterHit, threshold?: number): string {
  const lines: string[] = [];
  lines.push(`**Impact: ${hit.confidence_weighted.toFixed(1)}**`);
  lines.push(`Symbols at this line:`);
  for (const s of hit.symbols) {
    lines.push(`- \`${s}\``);
  }
  if (threshold !== undefined) {
    lines.push(`(warn threshold: ${threshold} — commit above this level triggers warning)`);
  }
  return lines.join("\n");
}
