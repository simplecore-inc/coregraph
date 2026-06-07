import type { DiffResult } from "../ipc/types";

export interface WarnThresholds {
  enabled: boolean;
  impactThreshold: number;
  inconsistencyCount: number;
}

/** Pure helper: decide whether a DiffResult should trigger the warning
 * given the current thresholds. Returns a short explanation string
 * when the warning fires, `null` otherwise.
 *
 * No VSCode dependency — fully unit-testable in plain Node. */
export function decideWarning(
  diff: DiffResult,
  th: WarnThresholds,
): string | null {
  if (!th.enabled) return null;
  const reasons: string[] = [];
  if (diff.total_confidence_weighted > th.impactThreshold) {
    reasons.push(
      `impact ${diff.total_confidence_weighted.toFixed(1)} > ${th.impactThreshold}`,
    );
  }
  if (diff.inconsistencies_introduced.length >= th.inconsistencyCount) {
    reasons.push(
      `${diff.inconsistencies_introduced.length} inconsistency(ies)`,
    );
  }
  if (reasons.length === 0) return null;
  return `⚠ High-impact commit: ${reasons.join(", ")}`;
}
