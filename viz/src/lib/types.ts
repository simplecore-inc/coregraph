/** A symbol node as emitted by `coregraph export --format json-graph`. */
export interface GraphNode {
  id: number
  name: string
  kind: string
  file: string
  spanStart: number
  spanEnd: number
}

/** A node enriched with dataset-level derived attributes. */
export interface NodeDatum extends GraphNode {
  /** Path relative to the dataset's common root directory. */
  rel: string
  /** First path segment under the common root ("(root)" for top-level files). */
  dir: string
  degIn: number
  degOut: number
  deg: number
}

/** An edge as emitted by the export (slimmed: trust/trust_model duplicates dropped). */
export interface GraphEdge {
  from: number
  to: number
  kind: string
  origin: string
  confidence: number
}

/** A fully parsed and indexed dataset. */
export interface ParsedGraph {
  nodes: NodeDatum[]
  edges: GraphEdge[]
  /** Full node count of the source graph, before any payload filtering. */
  totalNodes: number
  /** Full edge count of the source graph, before payload filtering (e.g. the
   * bridge omits `Resolves`). Shown as the project total so the loaded atlas
   * agrees with the picker. Falls back to `edges.length` for older exports. */
  totalEdges: number
  nodeById: ReadonlyMap<number, NodeDatum>
  /** Common directory prefix of all node file paths (with trailing slash, or ""). */
  root: string
  /** Top-level directory segments, sorted, for directory color mode. */
  dirs: string[]
  warnings: string[]
}

/** A renderable link object handed to the force graph (source/target get mutated into object refs by the engine). */
export interface LinkDatum {
  source: number | NodeDatum
  target: number | NodeDatum
  kind: string
  origin: string
  confidence: number
}

export interface VisibleGraph {
  nodes: NodeDatum[]
  links: LinkDatum[]
}

export interface ViewFilters {
  /** Enabled node kinds. */
  nodeKinds: ReadonlySet<string>
  /** Enabled edge kinds. */
  edgeKinds: ReadonlySet<string>
  /** Enabled analysis origins (trust tiers). */
  origins: ReadonlySet<string>
  minConfidence: number
  /** Drop nodes that have no visible incident edge. */
  hideIsolated: boolean
  /** Drop nodes with total degree below this (0 = off). */
  minDegree: number
  /** Drop the highest-degree hub nodes (above hubCap). */
  hideHubs: boolean
  /** Degree ceiling used when hideHubs is on (dataset p99, computed upstream). */
  hubCap: number
  /** Restrict to these top-level directories (null = all). */
  dirs: ReadonlySet<string> | null
}

export interface FocusSpec {
  id: number
  depth: number
}
