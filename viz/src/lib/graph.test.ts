import { describe, expect, it } from 'vitest'
import { computeVisible, incidentEdges, linkEndId, neighborIds } from './graph.ts'
import { parseDataset } from './parse.ts'
import type { ViewFilters } from './types.ts'

// Chain: f0 -Calls-> f1 -Calls-> f2, plus s3 -Imports-> f0 and a low-confidence
// Reference f2 -> s3, and an isolated constant c4.
const DATA = parseDataset({
  nodes: [
    { id: 0, name: 'f0', kind: 'Function', file: '/r/a.rs', span_start: 0, span_end: 1 },
    { id: 1, name: 'f1', kind: 'Function', file: '/r/b.rs', span_start: 0, span_end: 1 },
    { id: 2, name: 'f2', kind: 'Function', file: '/r/c.rs', span_start: 0, span_end: 1 },
    { id: 3, name: 's3', kind: 'Struct', file: '/r/d.rs', span_start: 0, span_end: 1 },
    { id: 4, name: 'c4', kind: 'Constant', file: '/r/e.rs', span_start: 0, span_end: 1 },
  ],
  edges: [
    { from: 0, to: 1, kind: 'Calls', origin: 'NameResolved', confidence: 0.85 },
    { from: 1, to: 2, kind: 'Calls', origin: 'NameResolved', confidence: 0.85 },
    { from: 3, to: 0, kind: 'Imports', origin: 'SyntaxMatched', confidence: 0.72 },
    { from: 2, to: 3, kind: 'References', origin: 'PatternMatched', confidence: 0.6 },
  ],
})

function filters(overrides: Partial<ViewFilters> = {}): ViewFilters {
  return {
    nodeKinds: new Set(['Function', 'Struct', 'Constant']),
    edgeKinds: new Set(['Calls', 'Imports', 'References']),
    origins: new Set(['NameResolved', 'SyntaxMatched', 'PatternMatched']),
    minConfidence: 0.7,
    hideIsolated: false,
    minDegree: 0,
    hideHubs: false,
    hubCap: Number.MAX_SAFE_INTEGER,
    dirs: null,
    ...overrides,
  }
}

describe('computeVisible', () => {
  it('keeps all nodes and confidence-passing links by default', () => {
    const visible = computeVisible(DATA, filters(), null)
    expect(visible.nodes).toHaveLength(5)
    // The 0.6-confidence Reference edge is filtered at 0.7.
    expect(visible.links).toHaveLength(3)
  })

  it('hides isolated nodes when requested', () => {
    const visible = computeVisible(DATA, filters({ hideIsolated: true }), null)
    expect(visible.nodes.map((n) => n.id).sort()).toEqual([0, 1, 2, 3])
  })

  it('filters by edge kind', () => {
    const visible = computeVisible(DATA, filters({ edgeKinds: new Set(['Calls']) }), null)
    expect(visible.links).toHaveLength(2)
  })

  it('filters by origin', () => {
    const visible = computeVisible(
      DATA,
      filters({ origins: new Set(['NameResolved']) }),
      null,
    )
    expect(visible.links).toHaveLength(2)
    expect(visible.links.every((l) => l.origin === 'NameResolved')).toBe(true)
  })

  it('drops edges touching hidden node kinds', () => {
    const visible = computeVisible(DATA, filters({ nodeKinds: new Set(['Function']) }), null)
    // s3-Imports->f0 disappears with the Struct kind.
    expect(visible.links).toHaveLength(2)
    expect(visible.nodes.every((n) => n.kind === 'Function')).toBe(true)
  })

  it('restricts to the ego network at depth 1', () => {
    const visible = computeVisible(DATA, filters(), { id: 1, depth: 1 })
    expect(visible.nodes.map((n) => n.id).sort()).toEqual([0, 1, 2])
    expect(visible.links).toHaveLength(2)
  })

  it('expands the ego network with depth', () => {
    const visible = computeVisible(DATA, filters(), { id: 2, depth: 2 })
    // depth 1: f1; depth 2: f0. s3 is reachable only via the filtered 0.6 edge.
    expect(visible.nodes.map((n) => n.id).sort()).toEqual([0, 1, 2])
  })

  it('includes intra-neighborhood edges between ego members', () => {
    const visible = computeVisible(DATA, filters({ minConfidence: 0 }), { id: 3, depth: 2 })
    // depth1: f0 (import target) + f2 (reference source); depth2: f1.
    expect(visible.nodes.map((n) => n.id).sort()).toEqual([0, 1, 2, 3])
    expect(visible.links).toHaveLength(4)
  })

  it('renders only the focus node at depth 0', () => {
    const visible = computeVisible(DATA, filters(), { id: 1, depth: 0 })
    expect(visible.nodes.map((n) => n.id)).toEqual([1])
    expect(visible.links).toHaveLength(0)
  })

  it('forces a hidden-kind focus node visible so its neighborhood survives', () => {
    // Struct s3 is filtered out by kind, yet focusing it must still surface its
    // real neighbors (f0 via Imports, f2 via References) rather than an orphan.
    const visible = computeVisible(
      DATA,
      filters({ nodeKinds: new Set(['Function']), minConfidence: 0 }),
      { id: 3, depth: 1 },
    )
    expect(visible.nodes.map((n) => n.id).sort()).toEqual([0, 2, 3])
    expect(visible.links).toHaveLength(2)
  })

  it('returns a lone node for a stale (unknown) focus id', () => {
    const visible = computeVisible(DATA, filters(), { id: 999, depth: 2 })
    expect(visible.nodes).toHaveLength(0)
    expect(visible.links).toHaveLength(0)
  })

  it('filters nodes below the degree floor', () => {
    // f1 has degree 2; everything else ≤ 2. minDegree 2 keeps f0/f1/f2/s3
    // shapes depending on edges: f0 deg 2, f1 deg 2, f2 deg 2, s3 deg 2, c4 deg 0.
    const visible = computeVisible(DATA, filters({ minDegree: 1 }), null)
    expect(visible.nodes.every((n) => n.deg >= 1)).toBe(true)
    expect(visible.nodes.some((n) => n.id === 4)).toBe(false)
  })

  it('hides hub nodes above the cap when hideHubs is on', () => {
    const visible = computeVisible(DATA, filters({ hideHubs: true, hubCap: 1 }), null)
    expect(visible.nodes.every((n) => n.deg <= 1)).toBe(true)
  })

  it('restricts to the directory filter', () => {
    // All fixture files live at the root ("(root)" dir).
    const all = computeVisible(DATA, filters({ dirs: new Set(['(root)']) }), null)
    expect(all.nodes.length).toBeGreaterThan(0)
    const none = computeVisible(DATA, filters({ dirs: new Set(['nope']) }), null)
    expect(none.nodes).toHaveLength(0)
    expect(none.links).toHaveLength(0)
  })
})

describe('incidentEdges', () => {
  it('returns directed incident edges with the counterpart node', () => {
    const incident = incidentEdges(DATA, filters(), 0)
    expect(incident).toHaveLength(2)
    const out = incident.find((i) => i.dir === 'out')
    const inn = incident.find((i) => i.dir === 'in')
    expect(out?.other.id).toBe(1)
    expect(inn?.other.id).toBe(3)
  })

  it('applies edge filters', () => {
    const incident = incidentEdges(DATA, filters({ minConfidence: 0 }), 3)
    expect(incident).toHaveLength(2)
  })
})

describe('neighborIds / linkEndId', () => {
  it('collects direct neighbors from links', () => {
    const visible = computeVisible(DATA, filters(), null)
    const neighbors = neighborIds(visible.links, 0)
    expect([...neighbors].sort()).toEqual([1, 3])
  })

  it('resolves both numeric and object endpoints', () => {
    expect(linkEndId(7)).toBe(7)
    const node = DATA.nodeById.get(0)
    expect(node).toBeDefined()
    if (node !== undefined) expect(linkEndId(node)).toBe(0)
  })

  it('collects neighbors when the engine has swapped ids for node objects', () => {
    // react-force-graph mutates link.source/target from ids into node refs;
    // neighborIds must still resolve them through linkEndId.
    const n0 = DATA.nodeById.get(0)
    const n1 = DATA.nodeById.get(1)
    expect(n0).toBeDefined()
    expect(n1).toBeDefined()
    if (n0 === undefined || n1 === undefined) return
    const mutatedLinks = [
      { source: n0, target: n1, kind: 'Calls', origin: 'NameResolved', confidence: 0.85 },
    ]
    expect([...neighborIds(mutatedLinks, 0)]).toEqual([1])
    expect([...neighborIds(mutatedLinks, 1)]).toEqual([0])
  })
})
