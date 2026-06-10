import type { NodeDatum, ParsedGraph } from './types.ts'

/**
 * Unit derivation for cluster/color grouping: module / package / crate-level
 * groups inferred purely from path structure (build manifests are not part
 * of the indexed graph, and naming conventions must not be hardcoded).
 *
 * Rule: a top-level directory is a CONTAINER — and gets expanded one level —
 * when it both spans several second-level directories and holds a meaningful
 * share of all nodes. This unfolds monorepo layouts (`crates/x`, `packages/y`,
 * `apps/z`) by their structure alone; ordinary top-level packages stay whole.
 */

/** A container must have at least this many second-level directories... */
const CONTAINER_MIN_CHILDREN = 2
/** ...and at least this share of all nodes (or this many nodes outright). */
const CONTAINER_MIN_SHARE = 0.12
const CONTAINER_MIN_NODES = 40

export interface UnitIndex {
  /** node id → unit label (e.g. "crates/cli", "viz", "(root)"). */
  unitOf: ReadonlyMap<number, string>
  /** Sorted distinct unit labels. */
  units: string[]
}

function topSegments(rel: string): { top: string; second: string | null; depth: number } {
  const parts = rel.split('/')
  if (parts.length <= 1) return { top: '(root)', second: null, depth: parts.length }
  return { top: parts[0], second: parts.length > 2 ? parts[1] : null, depth: parts.length }
}

export function deriveUnits(graph: ParsedGraph): UnitIndex {
  const topCount = new Map<string, number>()
  const topChildren = new Map<string, Set<string>>()
  for (const node of graph.nodes) {
    const { top, second } = topSegments(node.rel)
    topCount.set(top, (topCount.get(top) ?? 0) + 1)
    if (second !== null) {
      let children = topChildren.get(top)
      if (children === undefined) topChildren.set(top, (children = new Set()))
      children.add(second)
    }
  }
  const total = graph.nodes.length
  const expanded = new Set<string>()
  for (const [top, count] of topCount) {
    const children = topChildren.get(top)?.size ?? 0
    const bigEnough = count >= CONTAINER_MIN_NODES || count >= total * CONTAINER_MIN_SHARE
    if (children >= CONTAINER_MIN_CHILDREN && bigEnough) expanded.add(top)
  }

  const unitOf = new Map<number, string>()
  const unitSet = new Set<string>()
  for (const node of graph.nodes) {
    const { top, second } = topSegments(node.rel)
    const unit = expanded.has(top) && second !== null ? `${top}/${second}` : top
    unitOf.set(node.id, unit)
    unitSet.add(unit)
  }
  return { unitOf, units: [...unitSet].sort() }
}

/** Group key accessor used by both cluster anchoring and unit coloring. */
export function unitKeyOf(unitIndex: UnitIndex, node: NodeDatum): string {
  return unitIndex.unitOf.get(node.id) ?? node.dir
}
