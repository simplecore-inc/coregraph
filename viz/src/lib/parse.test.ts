import { describe, expect, it } from 'vitest'
import { GraphParseError, parseDataset } from './parse.ts'

const VALID = {
  nodes: [
    { id: 0, name: 'alpha', kind: 'Function', file: '/r/src/a.rs', span_start: 10, span_end: 50 },
    { id: 1, name: 'beta', kind: 'Struct', file: '/r/src/b.rs', span_start: 0, span_end: 5 },
    { id: 2, name: 'gamma', kind: 'Function', file: '/r/tests/c.rs', span_start: 1, span_end: 2 },
  ],
  edges: [
    { from: 0, to: 1, kind: 'Calls', origin: 'NameResolved', confidence: 0.85 },
    { from: 2, to: 0, kind: 'Calls', origin: 'SyntaxMatched', confidence: 0.72 },
  ],
}

describe('parseDataset', () => {
  it('rejects non-objects', () => {
    expect(() => parseDataset([])).toThrow(GraphParseError)
    expect(() => parseDataset('x')).toThrow(GraphParseError)
    expect(() => parseDataset(null)).toThrow(GraphParseError)
  })

  it('rejects documents without nodes/edges arrays', () => {
    expect(() => parseDataset({ nodes: 1, edges: [] })).toThrow(GraphParseError)
    expect(() => parseDataset({ nodes: [] })).toThrow(GraphParseError)
  })

  it('rejects documents with zero valid nodes', () => {
    expect(() => parseDataset({ nodes: [{ bogus: true }], edges: [] })).toThrow(GraphParseError)
  })

  it('parses a valid document and derives degrees, root and dirs', () => {
    const parsed = parseDataset(VALID)
    expect(parsed.nodes).toHaveLength(3)
    expect(parsed.edges).toHaveLength(2)
    expect(parsed.root).toBe('/r/')
    expect(parsed.nodeById.get(0)?.rel).toBe('src/a.rs')
    expect(parsed.nodeById.get(0)?.dir).toBe('src')
    expect(parsed.dirs).toEqual(['src', 'tests'])
    expect(parsed.nodeById.get(0)?.degOut).toBe(1)
    expect(parsed.nodeById.get(0)?.degIn).toBe(1)
    expect(parsed.nodeById.get(0)?.deg).toBe(2)
    expect(parsed.warnings).toEqual([])
  })

  it('skips dangling edges with a warning', () => {
    const parsed = parseDataset({
      ...VALID,
      edges: [...VALID.edges, { from: 0, to: 999, kind: 'Calls' }],
    })
    expect(parsed.edges).toHaveLength(2)
    expect(parsed.warnings.some((w) => w.includes('unknown node ids'))).toBe(true)
  })

  it('defaults edge origin and confidence, with trust fallback', () => {
    const parsed = parseDataset({
      nodes: VALID.nodes,
      edges: [
        { from: 0, to: 1, kind: 'Calls', trust: 'NameResolved' },
        { from: 1, to: 0, kind: 'Imports' },
      ],
    })
    expect(parsed.edges[0].origin).toBe('NameResolved')
    expect(parsed.edges[0].confidence).toBe(1)
    expect(parsed.edges[1].origin).toBe('Unknown')
  })

  it('falls back to the majority prefix when paths mix absolute and relative', () => {
    const parsed = parseDataset({
      nodes: [
        { id: 0, name: 'a', kind: 'Function', file: '/r/src/a.rs' },
        { id: 1, name: 'b', kind: 'Function', file: '/r/src/b.rs' },
        { id: 2, name: 'v', kind: 'ExternalPackage', file: 'virtual/pkg' },
      ],
      edges: [],
    })
    expect(parsed.root).toBe('/r/src/')
    expect(parsed.nodeById.get(0)?.rel).toBe('a.rs')
    // The outlier keeps its full path as the relative form.
    expect(parsed.nodeById.get(2)?.rel).toBe('virtual/pkg')
  })

  it('ignores empty file paths when deriving the root and marks them as root dir', () => {
    // ExternalPackage-style nodes have an empty file path; one such node must
    // not collapse the common root of an all-relative dataset.
    const parsed = parseDataset({
      nodes: [
        { id: 0, name: 'a', kind: 'Function', file: 'src/mod/a.rs' },
        { id: 1, name: 'b', kind: 'Function', file: 'src/mod/b.rs' },
        { id: 2, name: 'serde', kind: 'ExternalPackage', file: '' },
      ],
      edges: [],
    })
    expect(parsed.root).toBe('src/mod/')
    expect(parsed.nodeById.get(0)?.rel).toBe('a.rs')
    expect(parsed.nodeById.get(2)?.rel).toBe('')
    expect(parsed.nodeById.get(2)?.dir).toBe('(root)')
  })

  it('derives a directory for all-absolute disjoint paths', () => {
    const parsed = parseDataset({
      nodes: [
        { id: 0, name: 'a', kind: 'Function', file: '/repo/crates/a.rs' },
        { id: 1, name: 'b', kind: 'Function', file: '/repo/vscode/b.ts' },
      ],
      edges: [],
    })
    // Shared root collapses to "/repo/"; both keep a real top-level directory.
    expect(parsed.root).toBe('/repo/')
    expect(parsed.nodeById.get(0)?.dir).toBe('crates')
    expect(parsed.nodeById.get(1)?.dir).toBe('vscode')
  })

  it('drops duplicate node ids first-wins with a warning', () => {
    const parsed = parseDataset({
      nodes: [
        { id: 0, name: 'first', kind: 'Function', file: '/r/a.rs' },
        { id: 0, name: 'second', kind: 'Struct', file: '/r/b.rs' },
        { id: 1, name: 'other', kind: 'Function', file: '/r/c.rs' },
      ],
      edges: [],
    })
    expect(parsed.nodes).toHaveLength(2)
    expect(parsed.nodeById.get(0)?.name).toBe('first')
    expect(parsed.warnings.some((w) => w.includes('duplicate node id'))).toBe(true)
  })

  it('skips malformed rows but keeps the rest', () => {
    const parsed = parseDataset({
      nodes: [...VALID.nodes, { id: 'nan', name: 1 }, 42],
      edges: [...VALID.edges, { from: 'x' }, null],
    })
    expect(parsed.nodes).toHaveLength(3)
    expect(parsed.edges).toHaveLength(2)
    expect(parsed.warnings).toHaveLength(2)
  })
})
