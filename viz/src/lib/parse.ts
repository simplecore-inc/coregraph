import type { GraphEdge, NodeDatum, ParsedGraph } from './types.ts'
import { commonDirPrefix, isAbsolutePath, normalizeSlashes, topSegment } from './paths.ts'

/** Raised when the input is not a coregraph json-graph document. */
export class GraphParseError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'GraphParseError'
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function asString(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

/**
 * Parse a raw `coregraph export --format json-graph` document into an indexed dataset.
 * Tolerates the slimmed edge shape produced by scripts/embed-data.mjs (origin fallback
 * to `trust`, confidence default 1) and skips malformed rows with a warning instead of
 * failing the whole document.
 */
export function parseDataset(raw: unknown): ParsedGraph {
  if (!isRecord(raw)) {
    throw new GraphParseError('Expected a JSON object with "nodes" and "edges" arrays')
  }
  const rawNodes = raw.nodes
  const rawEdges = raw.edges
  if (!Array.isArray(rawNodes) || !Array.isArray(rawEdges)) {
    throw new GraphParseError('Missing "nodes" or "edges" array — is this a json-graph export?')
  }

  const warnings: string[] = []
  const nodes: NodeDatum[] = []
  const nodeById = new Map<number, NodeDatum>()
  let skippedNodes = 0
  let duplicateNodes = 0

  for (const entry of rawNodes) {
    if (!isRecord(entry)) {
      skippedNodes++
      continue
    }
    const id = asNumber(entry.id)
    const name = asString(entry.name)
    const kind = asString(entry.kind)
    const file = asString(entry.file)
    if (id === null || name === null || kind === null || file === null) {
      skippedNodes++
      continue
    }
    if (nodeById.has(id)) {
      // First-wins: a duplicate id would otherwise leave both nodes in the
      // array while degree counts attach only to whichever the index resolves.
      duplicateNodes++
      continue
    }
    const node: NodeDatum = {
      id,
      name,
      kind,
      file: normalizeSlashes(file),
      spanStart: asNumber(entry.span_start) ?? 0,
      spanEnd: asNumber(entry.span_end) ?? 0,
      rel: '',
      dir: '',
      degIn: 0,
      degOut: 0,
      deg: 0,
    }
    nodes.push(node)
    nodeById.set(id, node)
  }
  if (nodes.length === 0) {
    throw new GraphParseError('No valid nodes found in the document')
  }
  if (skippedNodes > 0) {
    warnings.push(`Skipped ${skippedNodes} malformed node row(s)`)
  }
  if (duplicateNodes > 0) {
    warnings.push(`Skipped ${duplicateNodes} duplicate node id(s)`)
  }

  const edges: GraphEdge[] = []
  let skippedEdges = 0
  let danglingEdges = 0
  for (const entry of rawEdges) {
    if (!isRecord(entry)) {
      skippedEdges++
      continue
    }
    const from = asNumber(entry.from)
    const to = asNumber(entry.to)
    const kind = asString(entry.kind)
    if (from === null || to === null || kind === null) {
      skippedEdges++
      continue
    }
    const fromNode = nodeById.get(from)
    const toNode = nodeById.get(to)
    if (fromNode === undefined || toNode === undefined) {
      danglingEdges++
      continue
    }
    edges.push({
      from,
      to,
      kind,
      origin: asString(entry.origin) ?? asString(entry.trust) ?? 'Unknown',
      confidence: asNumber(entry.confidence) ?? 1,
    })
    fromNode.degOut++
    toNode.degIn++
  }
  if (skippedEdges > 0) {
    warnings.push(`Skipped ${skippedEdges} malformed edge row(s)`)
  }
  if (danglingEdges > 0) {
    warnings.push(`Skipped ${danglingEdges} edge(s) referencing unknown node ids`)
  }

  // Empty file paths (e.g. ExternalPackage nodes) can't contribute to any
  // directory prefix and would collapse the common root to "" — exclude them.
  const files = nodes.map((n) => n.file).filter((f) => f !== '')
  let root = commonDirPrefix(files)
  if (root === '') {
    // Mixed absolute/relative inputs (some exports synthesize virtual paths)
    // defeat a global prefix; fall back to the majority group's prefix so
    // the bulk of the dataset still gets readable relative paths.
    const absolute: string[] = []
    const relative: string[] = []
    for (const file of files) (isAbsolutePath(file) ? absolute : relative).push(file)
    if (absolute.length > 0 && relative.length > 0) {
      root = commonDirPrefix(absolute.length >= relative.length ? absolute : relative)
    }
  }
  const dirSet = new Set<string>()
  for (const node of nodes) {
    node.rel = root !== '' && node.file.startsWith(root) ? node.file.slice(root.length) : node.file
    node.dir = topSegment(node.rel)
    node.deg = node.degIn + node.degOut
    dirSet.add(node.dir)
  }

  return {
    nodes,
    edges,
    nodeById,
    root,
    dirs: [...dirSet].sort(),
    warnings,
  }
}
