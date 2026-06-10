/** Typed client for the atlas bridge server (viz/server.mjs). */

declare global {
  interface Window {
    /** Per-process CSRF token injected into the HTML by the bridge server. */
    __BRIDGE_TOKEN__?: string
  }
}

function bridgeHeaders(json: boolean): HeadersInit {
  const headers: Record<string, string> = {}
  if (json) headers['Content-Type'] = 'application/json'
  if (typeof window.__BRIDGE_TOKEN__ === 'string') {
    headers['X-Bridge-Token'] = window.__BRIDGE_TOKEN__
  }
  return headers
}

export interface ProjectStatus {
  path: string
  loaded: boolean
  loading: boolean
  node_count: number
  edge_count: number
  idle_seconds: number
  active_queries: number
}

export interface ManagerStatus {
  projects: ProjectStatus[]
  max_loaded: number
  uptime_seconds: number
}

export interface BridgeStatus {
  bridge: 'ok'
  socket: string
  running: boolean
  manager: ManagerStatus | null
}

/** Raised for any bridge/daemon-level failure (HTTP error envelope). */
export class BridgeError extends Error {
  readonly status: number
  constructor(message: string, status: number) {
    super(message)
    this.name = 'BridgeError'
    this.status = status
  }
}

/** True when the page is served over HTTP(S) and the bridge API can exist. */
export function maybeBridged(): boolean {
  return window.location.protocol === 'http:' || window.location.protocol === 'https:'
}

async function jsonOrThrow<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let detail = response.statusText
    try {
      const body = (await response.json()) as { error?: string }
      if (typeof body.error === 'string') detail = body.error
    } catch {
      // Non-JSON error body — keep the HTTP status text.
    }
    throw new BridgeError(detail, response.status)
  }
  return (await response.json()) as T
}

export async function fetchStatus(signal?: AbortSignal): Promise<BridgeStatus> {
  return jsonOrThrow<BridgeStatus>(
    await fetch('/api/status', { headers: bridgeHeaders(false), signal }),
  )
}

export async function startDaemon(project: string, signal?: AbortSignal): Promise<BridgeStatus> {
  return jsonOrThrow<BridgeStatus>(
    await fetch('/api/daemon/start', {
      method: 'POST',
      headers: bridgeHeaders(true),
      body: JSON.stringify({ project }),
      signal,
    }),
  )
}

export type ResolveResult = { ok: true; path: string } | { ok: false; error: string }

export async function resolveProjectPath(path: string): Promise<ResolveResult> {
  return jsonOrThrow<ResolveResult>(
    await fetch(`/api/resolve?path=${encodeURIComponent(path)}`, {
      headers: bridgeHeaders(false),
    }),
  )
}

/** Force a full re-index: the daemon rebuilds the project graph from source. */
export async function reindexProject(
  project: string,
  signal: AbortSignal,
): Promise<{ reindexed: boolean; symbols: number; edges: number }> {
  return jsonOrThrow(
    await fetch('/api/reindex', {
      method: 'POST',
      headers: bridgeHeaders(true),
      body: JSON.stringify({ project }),
      signal,
    }),
  )
}

/** Fetch the full json-graph document for a project (may index it first). */
export async function fetchGraph(
  project: string,
  minConfidence: number,
  signal: AbortSignal,
): Promise<unknown> {
  const response = await fetch('/api/graph', {
    method: 'POST',
    headers: bridgeHeaders(true),
    body: JSON.stringify({ project, minConfidence }),
    signal,
  })
  return jsonOrThrow<unknown>(response)
}

// ── analysis endpoints ───────────────────────────────────────────────────

export interface ImpactTest {
  /** Graph node id (present in daemon responses — preferred for jumps). */
  id?: number
  name: string
  distance: number
  path_confidence: number
  file: string
}

export interface ImpactRisk {
  score: number
  level: string
  /** A grade label ("Critical"/"High"/…), not a number. */
  blast_radius: string
  module_count: number
  caller_count: number
  visibility_score: number
  caller_factor: number
  module_factor: number
  impact_kind_factor: number
  confidence_weighted_impact: number
  affected_tests: ImpactTest[]
}

export interface ImpactNode {
  id: number
  name: string
  kind: string
  file: string
}

export interface ImpactResult {
  symbol: string
  reachable: number
  edges: number
  depth: number
  transitive: boolean
  nodes: ImpactNode[]
  risk?: ImpactRisk
}

export async function fetchImpact(
  project: string,
  symbol: string,
  depth: number,
  signal: AbortSignal,
): Promise<ImpactResult> {
  return jsonOrThrow(
    await fetch('/api/impact', {
      method: 'POST',
      headers: bridgeHeaders(true),
      body: JSON.stringify({ project, symbol, depth }),
      signal,
    }),
  )
}

export interface OrphanEntry {
  name: string
  kind: string
  file: string
  /** 0-based. */
  line: number
  external_api: boolean
  is_test: boolean
}

export interface OrphansResult {
  count: number
  library_api_surface: number
  test_code: number
  likely_dead: number
  orphans: OrphanEntry[]
}

export async function fetchOrphans(
  project: string,
  options: { excludeTests: boolean; publicOnly: boolean },
  signal: AbortSignal,
): Promise<OrphansResult> {
  return jsonOrThrow(
    await fetch('/api/orphans', {
      method: 'POST',
      headers: bridgeHeaders(true),
      body: JSON.stringify({ project, ...options }),
      signal,
    }),
  )
}

export interface InconsistencySide {
  name: string
  file: string
  /** 0-based. */
  line: number
}

export interface InconsistencyReport {
  category: string
  shared_value?: string
  a?: InconsistencySide
  b?: InconsistencySide
  /** doc-drift reports use symbol/file/detail instead of a/b sides. */
  symbol?: string
  file?: string
  detail?: string
}

export interface InconsistenciesResult {
  count: number
  reports: InconsistencyReport[]
}

export async function fetchInconsistencies(
  project: string,
  options: { category?: string; excludeTests: boolean },
  signal: AbortSignal,
): Promise<InconsistenciesResult> {
  return jsonOrThrow(
    await fetch('/api/inconsistencies', {
      method: 'POST',
      headers: bridgeHeaders(true),
      body: JSON.stringify({ project, ...options }),
      signal,
    }),
  )
}

export interface DiffAffected {
  name: string
  file: string
  confidence: number
}

export interface DiffFile {
  file: string
  seed_symbols: string[]
  reachable_count: number
  confidence_weighted: number
  top_affected: DiffAffected[]
}

export interface DiffResult {
  base_ref: string
  changed_files: DiffFile[]
  total_reachable: number
  total_confidence_weighted: number
  inconsistencies_introduced: InconsistencyReport[]
  new_orphans: string[]
  git_operation_in_progress: boolean
  note?: string
}

export async function fetchDiff(
  project: string,
  baseRef: string,
  signal: AbortSignal,
): Promise<DiffResult> {
  return jsonOrThrow(
    await fetch('/api/diff', {
      method: 'POST',
      headers: bridgeHeaders(true),
      body: JSON.stringify({ project, baseRef }),
      signal,
    }),
  )
}

export interface SourceSnippet {
  file: string
  /** 0-based. */
  startLine: number
  endLine: number
  windowStart: number
  lines: string[]
  truncated: boolean
}

export async function fetchSource(
  project: string,
  file: string,
  spanStart: number,
  spanEnd: number,
  signal: AbortSignal,
): Promise<SourceSnippet> {
  return jsonOrThrow(
    await fetch('/api/source', {
      method: 'POST',
      headers: bridgeHeaders(true),
      body: JSON.stringify({ project, file, spanStart, spanEnd }),
      signal,
    }),
  )
}

const RECENTS_KEY = 'coregraph-atlas.recents'
const RECENTS_MAX = 6

export function recentProjects(): string[] {
  try {
    const raw = window.localStorage.getItem(RECENTS_KEY)
    if (raw === null) return []
    const parsed: unknown = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.filter((p): p is string => typeof p === 'string') : []
  } catch {
    // Corrupt or inaccessible storage — treat as empty, it is only a hint list.
    return []
  }
}

export function rememberProject(path: string): void {
  try {
    const next = [path, ...recentProjects().filter((p) => p !== path)].slice(0, RECENTS_MAX)
    window.localStorage.setItem(RECENTS_KEY, JSON.stringify(next))
  } catch {
    // Storage may be unavailable (private mode); recents are best-effort.
  }
}
