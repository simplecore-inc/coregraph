import { describe, expect, it } from 'vitest'
import { deriveUnits } from './units.ts'
import { parseDataset } from './parse.ts'

function nodesUnder(paths: string[]): { id: number; name: string; kind: string; file: string }[] {
  return paths.map((p, id) => ({ id, name: `n${id}`, kind: 'Function', file: `/repo/${p}` }))
}

describe('deriveUnits', () => {
  it('expands a container directory with several children and enough nodes', () => {
    // 60 nodes under crates/{a,b,c}, 5 under viz — crates is a container.
    const paths: string[] = []
    for (let i = 0; i < 20; i++) paths.push(`crates/a/src/f${i}.rs`)
    for (let i = 0; i < 20; i++) paths.push(`crates/b/src/f${i}.rs`)
    for (let i = 0; i < 20; i++) paths.push(`crates/c/src/f${i}.rs`)
    for (let i = 0; i < 5; i++) paths.push(`viz/src/f${i}.ts`)
    const graph = parseDataset({ nodes: nodesUnder(paths), edges: [] })
    const { unitOf, units } = deriveUnits(graph)
    expect(units).toContain('crates/a')
    expect(units).toContain('crates/b')
    expect(units).toContain('viz')
    expect(units).not.toContain('crates')
    const sample = graph.nodes.find((n) => n.rel.startsWith('crates/b/'))
    expect(sample).toBeDefined()
    if (sample !== undefined) expect(unitOf.get(sample.id)).toBe('crates/b')
  })

  it('keeps small or single-child top dirs whole', () => {
    const paths: string[] = []
    for (let i = 0; i < 50; i++) paths.push(`src/mod${i % 3}/f${i}.rs`)
    for (let i = 0; i < 4; i++) paths.push(`tools/x/f${i}.rs`)
    const graph = parseDataset({ nodes: nodesUnder(paths), edges: [] })
    const { units } = deriveUnits(graph)
    // src expands (3 children, dominant share); tools stays whole (too small).
    expect(units).toContain('src/mod0')
    expect(units).toContain('tools')
    expect(units).not.toContain('tools/x')
  })

  it('labels files directly under an expanded container by the container', () => {
    const paths: string[] = ['main.ts'] // pins the common root above packages/
    for (let i = 0; i < 30; i++) paths.push(`packages/a/f${i}.ts`)
    for (let i = 0; i < 30; i++) paths.push(`packages/b/f${i}.ts`)
    paths.push('packages/README.md')
    const graph = parseDataset({ nodes: nodesUnder(paths), edges: [] })
    const { unitOf } = deriveUnits(graph)
    const readme = graph.nodes.find((n) => n.rel === 'packages/README.md')
    expect(readme).toBeDefined()
    if (readme !== undefined) expect(unitOf.get(readme.id)).toBe('packages')
  })

  it('handles root-level files', () => {
    const graph = parseDataset({
      nodes: nodesUnder(['main.rs', 'lib.rs']),
      edges: [],
    })
    const { units } = deriveUnits(graph)
    expect(units).toEqual(['(root)'])
  })
})
