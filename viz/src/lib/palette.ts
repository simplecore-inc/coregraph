/**
 * Color system for the atlas. Known coregraph symbol/edge kinds get curated,
 * dark-background-tuned colors; unknown kinds fall back to a deterministic
 * golden-angle hue so any future export vocabulary still renders distinctly.
 * All outputs are hex/rgba strings, the only formats the WebGL renderer's
 * color parser accepts (CSS color-mix etc. is not available there).
 */

const NODE_KIND_COLORS: Readonly<Record<string, string>> = {
  Function: '#ffb454',
  Method: '#ff8a5c',
  Struct: '#4fc1e9',
  Class: '#43c6ac',
  Interface: '#9d7cd8',
  Trait: '#c792ea',
  Enum: '#7aa2f7',
  EnumVariant: '#a8bffb',
  Constant: '#d7ba7d',
  TypeAlias: '#56d4dd',
  Field: '#98c379',
  Variable: '#8ec07c',
  Module: '#61afef',
  Namespace: '#61afef',
  File: '#6b7d90',
  ExternalPackage: '#8a91f8',
  ConfigKey: '#c3a6ff',
  StringLiteral: '#6a9955',
  DocComment: '#54707f',
  DocSection: '#46606e',
}

const EDGE_KIND_COLORS: Readonly<Record<string, string>> = {
  Calls: '#ffb454',
  Imports: '#61afef',
  References: '#56d4dd',
  TypeOf: '#98c379',
  Implements: '#c792ea',
  Extends: '#ff8a5c',
  GenericParam: '#9d7cd8',
  Resolves: '#3b4a5e',
  Contains: '#2e3b4e',
  BelongsTo: '#2e3b4e',
  Documents: '#46586a',
  DescribedIn: '#46586a',
  Mentions: '#46586a',
  Configures: '#7d6aa6',
  ApiPathMatch: '#b3675e',
  EnumValueMatch: '#b3675e',
}

/** Node kinds hidden by default: structural/textual layers that hairball the code view. */
export const DEFAULT_HIDDEN_NODE_KINDS: readonly string[] = [
  'File',
  'DocComment',
  'DocSection',
  'StringLiteral',
  'ConfigKey',
]

/** Edge kinds hidden by default: containment/resolution plumbing, not code relations. */
export const DEFAULT_HIDDEN_EDGE_KINDS: readonly string[] = [
  'Contains',
  'BelongsTo',
  'Resolves',
  'Documents',
  'DescribedIn',
  'Mentions',
]

/** Display order for analysis origins, highest trust first. */
export const ORIGIN_ORDER: readonly string[] = [
  'CompilerDerived',
  'NameResolved',
  'SyntaxMatched',
  'PatternMatched',
  'ConventionInferred',
]

function hashString(value: string): number {
  let hash = 0
  for (let i = 0; i < value.length; i++) {
    hash = (hash * 31 + value.charCodeAt(i)) | 0
  }
  return hash >>> 0
}

function hslToHex(h: number, s: number, l: number): string {
  const sn = s / 100
  const ln = l / 100
  const a = sn * Math.min(ln, 1 - ln)
  const channel = (n: number): string => {
    const k = (n + h / 30) % 12
    const value = ln - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))
    return Math.round(value * 255)
      .toString(16)
      .padStart(2, '0')
  }
  return `#${channel(0)}${channel(8)}${channel(4)}`
}

/** Deterministic fallback color via golden-angle hue spacing. */
function goldenColor(seed: number, saturation: number, lightness: number): string {
  return hslToHex((seed * 137.508) % 360, saturation, lightness)
}

export function nodeKindColor(kind: string): string {
  return NODE_KIND_COLORS[kind] ?? goldenColor(hashString(kind), 62, 64)
}

export function edgeKindColor(kind: string): string {
  return EDGE_KIND_COLORS[kind] ?? goldenColor(hashString(kind), 38, 46)
}

/** Stable palette for directory color mode, assigned by sorted index. */
export function dirColor(index: number): string {
  return goldenColor(index + 1, 58, 62)
}

const rgbaCache = new Map<string, string>()

/**
 * Hex color → rgba() string with the given alpha. Used to fade
 * non-neighborhood elements behind a selection.
 */
export function withAlpha(hex: string, alpha: number): string {
  const key = `${hex}/${alpha}`
  const cached = rgbaCache.get(key)
  if (cached !== undefined) return cached
  const r = parseInt(hex.slice(1, 3), 16)
  const g = parseInt(hex.slice(3, 5), 16)
  const b = parseInt(hex.slice(5, 7), 16)
  const out = `rgba(${r},${g},${b},${alpha})`
  rgbaCache.set(key, out)
  return out
}
