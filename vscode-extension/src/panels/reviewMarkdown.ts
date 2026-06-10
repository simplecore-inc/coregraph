import type { DiffResult } from "../ipc/types";

/** Build a Markdown comment body loosely inspired by `coregraph
 * review`'s `render_comment`, but with a distinct table layout. The
 * preview is synthesized entirely in the extension from the
 * git-enriched DiffResult:
 *
 * - changed_files.length            → "Changed files"
 * - sum of seed_symbols[]           → "Touched symbols"
 * - total_reachable                 → "Reach"
 * - total_confidence_weighted       → "Impact"
 *
 * Note this diverges from the CLI table: the CLI labels the row
 * "Reachable symbols" (we use "Reach"), has no "Impact" row, and
 * lists each touched symbol with its kind (`- name _Kind_ — file`),
 * whereas this preview omits the kind.
 *
 * The CLI currently omits inconsistencies from its comment body;
 * this preview does the same. */
export function buildReviewMarkdown(diff: DiffResult): string {
  const lines: string[] = [];

  lines.push("## CoreGraph impact analysis");
  lines.push("");

  const touched: string[] = [];
  for (const f of diff.changed_files) {
    for (const name of f.seed_symbols) {
      touched.push(`- \`${name}\` — \`${f.file}\``);
    }
  }

  lines.push(
    `Diffing \`${diff.base_ref}..working\` (depth 3):`,
  );
  lines.push("");
  lines.push("| Metric | Count |");
  lines.push("|--------|-------|");
  lines.push(`| Changed files | **${diff.changed_files.length}** |`);
  lines.push(`| Touched symbols | **${touched.length}** |`);
  lines.push(`| Reach | **${diff.total_reachable}** |`);
  lines.push(`| Impact | **${diff.total_confidence_weighted.toFixed(1)}** |`);
  lines.push("");

  if (touched.length === 0) {
    lines.push("_No symbols touched — refactor or doc-only change._");
  } else {
    lines.push("### Touched symbols");
    lines.push("");
    for (const t of touched.slice(0, 25)) {
      lines.push(t);
    }
    if (touched.length > 25) {
      lines.push("");
      lines.push(`…and ${touched.length - 25} more.`);
    }
  }
  lines.push("");
  lines.push("<sub>Preview rendered by the CoreGraph VSCode extension (not `coregraph review`).</sub>");

  return lines.join("\n");
}
