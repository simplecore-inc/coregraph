import { describe, expect, it } from 'vitest'
import { fuzzyScore, searchNodes } from './fuzzy.ts'
import type { NodeDatum } from './types.ts'

function node(id: number, name: string, rel: string, deg = 0): NodeDatum {
  return {
    id,
    name,
    kind: 'Function',
    file: `/r/${rel}`,
    spanStart: 0,
    spanEnd: 0,
    rel,
    dir: 'src',
    degIn: 0,
    degOut: deg,
    deg,
  }
}

describe('fuzzyScore', () => {
  it('returns null when not a subsequence', () => {
    expect(fuzzyScore('xyz', 'build_graph')).toBeNull()
  })

  it('returns null for an empty query', () => {
    expect(fuzzyScore('', 'anything')).toBeNull()
  })

  it('ranks exact substring above scattered subsequence', () => {
    const exact = fuzzyScore('graph', 'build_graph')
    const scattered = fuzzyScore('graph', 'g_r_a_p_h_x')
    expect(exact).not.toBeNull()
    expect(scattered).not.toBeNull()
    expect(exact!).toBeGreaterThan(scattered!)
  })

  it('prefers prefix matches over mid-string matches', () => {
    const prefix = fuzzyScore('build', 'build_graph')
    const mid = fuzzyScore('build', 'rebuild_graph')
    expect(prefix!).toBeGreaterThan(mid!)
  })

  it('is case-insensitive', () => {
    expect(fuzzyScore('BUILD', 'build_graph')).not.toBeNull()
  })

  it('treats hyphen, underscore and space as interchangeable separators', () => {
    // Searching the CLI format name "json-graph" must find json_graph_string.
    expect(fuzzyScore('json-graph', 'json_graph_string')).not.toBeNull()
    expect(fuzzyScore('json graph', 'json_graph_string')).not.toBeNull()
    expect(fuzzyScore('json_graph', 'json-graph-cli')).not.toBeNull()
    // A separator query char still must align with a separator, not a letter.
    expect(fuzzyScore('json-graph', 'jsonXgraph')).toBeNull()
  })

  it('handles an empty target without throwing', () => {
    expect(fuzzyScore('x', '')).toBeNull()
  })

  it('does not throw on text whose lowercasing changes length', () => {
    // Turkish dotted capital İ lowercases to two code units; the boundary
    // check must stay index-safe and not crash.
    expect(() => fuzzyScore('i', 'İstanbul')).not.toThrow()
    expect(() => fuzzyScore('stan', 'İstanbul')).not.toThrow()
  })

  it('still detects camelCase boundaries for ASCII identifiers', () => {
    const atBoundary = fuzzyScore('g', 'buildGraph')
    const midWord = fuzzyScore('u', 'buildGraph')
    expect(atBoundary).not.toBeNull()
    expect(midWord).not.toBeNull()
    // 'G' starts a camelCase word; 'u' is mid-word, so the boundary bonus ranks it higher.
    expect(atBoundary!).toBeGreaterThan(midWord!)
  })
})

describe('searchNodes', () => {
  const nodes = [
    node(0, 'build_graph', 'graph/build.rs', 10),
    node(1, 'GraphBuilder', 'builder.rs', 2),
    node(2, 'unrelated', 'other.rs'),
    node(3, 'rebuild_graph_cache', 'cache.rs'),
  ]

  it('returns empty for blank queries', () => {
    expect(searchNodes(nodes, '   ', 10)).toEqual([])
  })

  it('finds and ranks name matches first', () => {
    const hits = searchNodes(nodes, 'build_graph', 10)
    expect(hits.length).toBeGreaterThanOrEqual(2)
    expect(hits[0].node.id).toBe(0)
  })

  it('matches against file paths too', () => {
    const hits = searchNodes(nodes, 'other.rs', 10)
    expect(hits.some((h) => h.node.id === 2)).toBe(true)
  })

  it('honors the limit', () => {
    expect(searchNodes(nodes, 'r', 2)).toHaveLength(2)
  })
})
