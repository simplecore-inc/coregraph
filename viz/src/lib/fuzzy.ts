import type { NodeDatum } from './types.ts'

// Word separators are treated as one equivalence class so a query can mix
// "-", "_", ".", "/", ":" and spaces freely — searching "json-graph" still
// matches the symbol "json_graph_string" (a very common code-search habit).
const SEPARATORS = new Set(['-', '_', '.', ':', '/', ' '])

function charsMatch(a: string, b: string): boolean {
  return a === b || (SEPARATORS.has(a) && SEPARATORS.has(b))
}

/** Start index of a separator-insensitive contiguous match of `q` in `t`, or -1. */
function contiguousIndex(t: string, q: string): number {
  const last = t.length - q.length
  for (let start = 0; start <= last; start++) {
    let ok = true
    for (let k = 0; k < q.length; k++) {
      if (!charsMatch(q[k], t[start + k])) {
        ok = false
        break
      }
    }
    if (ok) return start
  }
  return -1
}

/**
 * Subsequence fuzzy score. Higher is better; null when `query` is not a
 * subsequence of `text`. Rewards consecutive runs, word-boundary hits and
 * early matches; penalizes gaps. Separators are interchangeable (see above).
 */
export function fuzzyScore(query: string, text: string): number | null {
  if (query.length === 0) return null
  const q = query.toLowerCase()
  const t = text.toLowerCase()
  // camelCase detection needs the original casing, but only when lowercasing
  // preserved indices (Turkish İ, German ß etc. change length and would
  // misalign `idx` against `text`); the separator check works on `t` directly.
  const sameLength = t.length === text.length

  // A contiguous (separator-insensitive) match dominates scattered ones.
  const sub = contiguousIndex(t, q)
  if (sub !== -1) {
    let score = 100 + q.length * 4 - sub * 0.5
    if (sub === 0) score += 20
    else if (isBoundary(t, text, sameLength, sub)) score += 10
    if (t.length === q.length) score += 30
    return score
  }

  let score = 0
  let ti = 0
  let prev = -2
  for (let qi = 0; qi < q.length; qi++) {
    let idx = -1
    for (let j = ti; j < t.length; j++) {
      if (charsMatch(q[qi], t[j])) {
        idx = j
        break
      }
    }
    if (idx === -1) return null
    if (idx === prev + 1) score += 4
    else score -= Math.min(idx - ti, 12) * 0.25
    if (isBoundary(t, text, sameLength, idx)) score += 3
    prev = idx
    ti = idx + 1
  }
  return score
}

/**
 * `lower` is the lowercased text (index-aligned with `idx`); `original` is the
 * source text used only for camelCase detection when `sameLength` holds.
 */
function isBoundary(lower: string, original: string, sameLength: boolean, idx: number): boolean {
  if (idx === 0) return true
  const prevLower = lower[idx - 1]
  if (
    prevLower === '_' ||
    prevLower === '/' ||
    prevLower === '.' ||
    prevLower === ':' ||
    prevLower === '-'
  ) {
    return true
  }
  if (!sameLength) return false
  // camelCase boundary (original casing, indices guaranteed aligned).
  const prev = original[idx - 1]
  const cur = original[idx]
  return cur >= 'A' && cur <= 'Z' && prev >= 'a' && prev <= 'z'
}

export interface SearchHit {
  node: NodeDatum
  score: number
}

/** Rank nodes against a query: symbol names weigh more than file paths. */
export function searchNodes(
  nodes: readonly NodeDatum[],
  query: string,
  limit: number,
): SearchHit[] {
  const trimmed = query.trim()
  if (trimmed === '') return []
  const hits: SearchHit[] = []
  for (const node of nodes) {
    const byName = fuzzyScore(trimmed, node.name)
    const byPath = fuzzyScore(trimmed, node.rel)
    let score: number | null = null
    if (byName !== null) score = byName * 2
    if (byPath !== null) score = Math.max(score ?? -Infinity, byPath)
    if (score !== null) {
      // Well-connected symbols are more likely to be what the user means.
      hits.push({ node, score: score + Math.min(node.deg, 40) * 0.05 })
    }
  }
  hits.sort((a, b) => b.score - a.score || a.node.name.length - b.node.name.length)
  return hits.slice(0, limit)
}
