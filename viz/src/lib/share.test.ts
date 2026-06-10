// btoa/atob/TextEncoder are Node globals (18+), so the default node env works.
import { describe, expect, it } from 'vitest'
import { decodeShareState, encodeShareState } from './share.ts'

describe('share state codec', () => {
  it('round-trips a full state', () => {
    const state = {
      project: '/Users/me/proj',
      symbol: 'build_graph',
      focus: true,
      depth: 2,
      minConfidence: 0.7,
      colorMode: 'dir' as const,
      minDegree: 3,
      hideHubs: true,
      dir: 'crates',
      hiddenNodeKinds: ['File', 'DocComment'],
      hiddenEdgeKinds: ['Contains'],
    }
    const decoded = decodeShareState(`#${encodeShareState(state)}`)
    expect(decoded).toEqual(state)
  })

  it('round-trips non-ASCII project paths', () => {
    const state = { project: '/Users/태환/작업/프로젝트' }
    expect(decodeShareState(`#${encodeShareState(state)}`)).toEqual(state)
  })

  it('ignores garbage hashes', () => {
    expect(decodeShareState('#v=!!!notbase64')).toBeNull()
    expect(decodeShareState('#other=1')).toBeNull()
    expect(decodeShareState('')).toBeNull()
    expect(decodeShareState('#v=' + btoa('[1,2]').replace(/=+$/, ''))).toBeNull()
  })
})
