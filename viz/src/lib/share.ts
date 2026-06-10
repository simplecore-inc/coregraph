/**
 * Shareable view state, encoded into the URL hash (#v=...) so a teammate
 * opening the link lands on the same project, symbol and filters. Compact
 * JSON → base64url; unknown/garbled hashes are ignored.
 */

export interface ShareState {
  project?: string
  /** Selected symbol name (resolved back by name after load). */
  symbol?: string
  focus?: boolean
  depth?: number
  minConfidence?: number
  colorMode?: 'kind' | 'dir' | 'unit'
  clusterBy?: 'none' | 'unit'
  minDegree?: number
  hideHubs?: boolean
  dir?: string
  hiddenNodeKinds?: string[]
  hiddenEdgeKinds?: string[]
}

function toBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text)
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function fromBase64Url(encoded: string): string {
  const padded = encoded.replace(/-/g, '+').replace(/_/g, '/')
  const binary = atob(padded)
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0))
  return new TextDecoder().decode(bytes)
}

export function encodeShareState(state: ShareState): string {
  return `v=${toBase64Url(JSON.stringify(state))}`
}

export function decodeShareState(hash: string): ShareState | null {
  const match = /(?:^|[#&])v=([A-Za-z0-9_-]+)/.exec(hash)
  if (match === null) return null
  try {
    const parsed: unknown = JSON.parse(fromBase64Url(match[1]))
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return null
    return parsed as ShareState
  } catch {
    // A hand-edited or truncated hash is not an error condition — just no state.
    return null
  }
}
