import { describe, expect, it } from 'vitest'
import {
  buildAdjacency,
  buildLocator,
  hopDistances,
  pathOverlayLinks,
  resolveSymbol,
  shortestPath,
} from './analysis.ts'
import { parseDataset } from './parse.ts'
import type { ViewFilters } from './types.ts'

// f0(a) -Calls-> f1(a) -Calls-> f2(b), s3(b) -Imports-> f0, isolated c4(b).
const DATA = parseDataset({
  nodes: [
    { id: 0, name: 'f0', kind: 'Function', file: '/r/a/one.rs' },
    { id: 1, name: 'f1', kind: 'Function', file: '/r/a/two.rs' },
    { id: 2, name: 'f2', kind: 'Function', file: '/r/b/three.rs' },
    { id: 3, name: 's3', kind: 'Struct', file: '/r/b/four.rs' },
    { id: 4, name: 'dup', kind: 'Constant', file: '/r/a/five.rs' },
    { id: 5, name: 'dup', kind: 'Constant', file: '/r/b/six.rs' },
  ],
  edges: [
    { from: 0, to: 1, kind: 'Calls', origin: 'NameResolved', confidence: 0.85 },
    { from: 1, to: 2, kind: 'Calls', origin: 'NameResolved', confidence: 0.85 },
    { from: 3, to: 0, kind: 'Imports', origin: 'SyntaxMatched', confidence: 0.72 },
  ],
})

function filters(overrides: Partial<ViewFilters> = {}): ViewFilters {
  return {
    nodeKinds: new Set(['Function', 'Struct', 'Constant']),
    edgeKinds: new Set(['Calls', 'Imports']),
    origins: new Set(['NameResolved', 'SyntaxMatched']),
    minConfidence: 0,
    hideIsolated: false,
    minDegree: 0,
    hideHubs: false,
    hubCap: Number.MAX_SAFE_INTEGER,
    dirs: null,
    ...overrides,
  }
}

describe('buildLocator / resolveSymbol', () => {
  const locator = buildLocator(DATA)

  it('resolves by exact file+name', () => {
    expect(resolveSymbol(locator, 'f0', '/r/a/one.rs')?.id).toBe(0)
  })

  it('resolves a unique name without a file', () => {
    expect(resolveSymbol(locator, 's3')?.id).toBe(3)
  })

  it('refuses ambiguous names without a matching file', () => {
    expect(resolveSymbol(locator, 'dup')).toBeUndefined()
    expect(resolveSymbol(locator, 'dup', '/r/b/six.rs')?.id).toBe(5)
  })

  it('returns undefined for unknown symbols', () => {
    expect(resolveSymbol(locator, 'nope', '/r/a/one.rs')).toBeUndefined()
  })
})

describe('hopDistances', () => {
  it('measures hops within the membership set', () => {
    const adjacency = buildAdjacency(DATA.edges)
    const within = new Set([0, 1, 2, 3])
    const distance = hopDistances(adjacency, 0, within)
    expect(distance.get(0)).toBe(0)
    expect(distance.get(1)).toBe(1)
    expect(distance.get(2)).toBe(2)
    expect(distance.get(3)).toBe(1)
  })

  it('does not escape the membership set', () => {
    const adjacency = buildAdjacency(DATA.edges)
    const distance = hopDistances(adjacency, 0, new Set([0, 2]))
    // 2 is only reachable through 1, which is outside the set.
    expect(distance.has(2)).toBe(false)
  })
})

describe('shortestPath', () => {
  it('finds the hop-minimal chain', () => {
    expect(shortestPath(DATA, filters(), 3, 2)).toEqual([3, 0, 1, 2])
  })

  it('is direction-agnostic', () => {
    expect(shortestPath(DATA, filters(), 2, 3)).toEqual([2, 1, 0, 3])
  })

  it('respects edge-kind filters', () => {
    expect(shortestPath(DATA, filters({ edgeKinds: new Set(['Calls']) }), 3, 2)).toBeNull()
  })

  it('handles the trivial case', () => {
    expect(shortestPath(DATA, filters(), 1, 1)).toEqual([1])
  })

  it('returns null when no path exists', () => {
    expect(shortestPath(DATA, filters(), 0, 4)).toBeNull()
  })

  it('builds overlay links for consecutive pairs', () => {
    const links = pathOverlayLinks([3, 0, 1])
    expect(links).toHaveLength(2)
    expect(links[0]).toMatchObject({ source: 3, target: 0, kind: '__path' })
  })
})

