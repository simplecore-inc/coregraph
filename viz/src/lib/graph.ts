import type {
  FocusSpec,
  GraphEdge,
  LinkDatum,
  NodeDatum,
  ParsedGraph,
  ViewFilters,
  VisibleGraph,
} from './types.ts'

function edgePasses(edge: GraphEdge, filters: ViewFilters): boolean {
  return (
    filters.edgeKinds.has(edge.kind) &&
    filters.origins.has(edge.origin) &&
    edge.confidence >= filters.minConfidence
  )
}

function toLink(edge: GraphEdge): LinkDatum {
  // Fresh objects per computation: the force engine mutates source/target into
  // node references, so the master edge list must never be handed over directly.
  return {
    source: edge.from,
    target: edge.to,
    kind: edge.kind,
    origin: edge.origin,
    confidence: edge.confidence,
  }
}

/**
 * Apply the active filters (and optional ego focus) to the dataset.
 * Node objects keep their identity across calls so the force engine
 * preserves layout positions when filters change.
 */
export function computeVisible(
  graph: ParsedGraph,
  filters: ViewFilters,
  focus: FocusSpec | null,
): VisibleGraph {
  const kindVisible = (id: number): boolean => {
    // The focused symbol is always visible even if its kind is filtered out,
    // so isolating e.g. a File node still yields its real neighborhood rather
    // than a single orphan with every edge dropped.
    if (focus !== null && id === focus.id) return true
    const node = graph.nodeById.get(id)
    if (node === undefined || !filters.nodeKinds.has(node.kind)) return false
    if (node.deg < filters.minDegree) return false
    if (filters.hideHubs && node.deg > filters.hubCap) return false
    if (filters.dirs !== null && !filters.dirs.has(node.dir)) return false
    return true
  }

  // Single pass over edges: visibility filter + adjacency for the focus BFS.
  const activeEdges: GraphEdge[] = []
  const adjacency = new Map<number, number[]>()
  for (const edge of graph.edges) {
    if (!edgePasses(edge, filters)) continue
    if (!kindVisible(edge.from) || !kindVisible(edge.to)) continue
    activeEdges.push(edge)
    if (focus !== null) {
      let fromList = adjacency.get(edge.from)
      if (fromList === undefined) adjacency.set(edge.from, (fromList = []))
      fromList.push(edge.to)
      let toList = adjacency.get(edge.to)
      if (toList === undefined) adjacency.set(edge.to, (toList = []))
      toList.push(edge.from)
    }
  }

  if (focus !== null) {
    const kept = new Set<number>([focus.id])
    let frontier = [focus.id]
    for (let depth = 0; depth < focus.depth && frontier.length > 0; depth++) {
      const next: number[] = []
      for (const id of frontier) {
        const neighbors = adjacency.get(id)
        if (neighbors === undefined) continue
        for (const other of neighbors) {
          if (!kept.has(other)) {
            kept.add(other)
            next.push(other)
          }
        }
      }
      frontier = next
    }
    const nodes: NodeDatum[] = []
    for (const id of kept) {
      const node = graph.nodeById.get(id)
      if (node !== undefined) nodes.push(node)
    }
    const links: LinkDatum[] = []
    for (const edge of activeEdges) {
      if (kept.has(edge.from) && kept.has(edge.to)) links.push(toLink(edge))
    }
    return { nodes, links }
  }

  const links = activeEdges.map(toLink)
  let nodes: NodeDatum[]
  if (filters.hideIsolated) {
    const connected = new Set<number>()
    for (const edge of activeEdges) {
      connected.add(edge.from)
      connected.add(edge.to)
    }
    nodes = graph.nodes.filter((n) => connected.has(n.id))
  } else {
    nodes = graph.nodes.filter((n) => kindVisible(n.id))
  }
  return { nodes, links }
}

export interface IncidentEdge {
  edge: GraphEdge
  /** Direction relative to the inspected node. */
  dir: 'out' | 'in'
  other: NodeDatum
}

/**
 * Edges incident to one node under the active edge filters (node-kind and
 * focus restrictions intentionally ignored so the detail panel always shows
 * the full neighborhood of the selected symbol).
 */
export function incidentEdges(
  graph: ParsedGraph,
  filters: ViewFilters,
  id: number,
): IncidentEdge[] {
  const result: IncidentEdge[] = []
  for (const edge of graph.edges) {
    if (edge.from !== id && edge.to !== id) continue
    if (!edgePasses(edge, filters)) continue
    const dir = edge.from === id ? 'out' : 'in'
    const other = graph.nodeById.get(dir === 'out' ? edge.to : edge.from)
    if (other !== undefined) result.push({ edge, dir, other })
  }
  return result
}

/** Ids of nodes directly linked to `id` in the currently visible link set. */
export function neighborIds(links: readonly LinkDatum[], id: number): ReadonlySet<number> {
  const result = new Set<number>()
  for (const link of links) {
    const source = linkEndId(link.source)
    const target = linkEndId(link.target)
    if (source === id) result.add(target)
    else if (target === id) result.add(source)
  }
  return result
}

/** The force engine swaps numeric endpoint ids for node object references after init. */
export function linkEndId(end: number | NodeDatum): number {
  return typeof end === 'number' ? end : end.id
}
