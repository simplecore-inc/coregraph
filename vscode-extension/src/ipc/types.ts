/** The outer wire envelope the Rust daemon returns on every IPC reply.
 *
 * Note: this is intentionally non-generic. The inner payload type is
 * recovered by the caller via `JSON.parse(body) as T`, not by the
 * envelope type itself — `body` is always a JSON-encoded string on the
 * wire. See `IpcClient.request<T>()`.
 */
export interface Response {
  ok: boolean;
  /** Body is JSON-encoded as a STRING on the wire (see Rust `Response.body`). */
  body: string;
  error: string | null;
}

/** `reindex` method response body (inner — parsed from `Response.body`). */
export interface ReindexResult {
  reindexed: boolean;
  mode: 'fast' | 'full';
  elapsed_ms: number;
  /** Present only for mode=full. Node count after rebuild. */
  node_count?: number;
  edge_count?: number;
  /**
   * Telemetry fields present for mode=fast.
   *
   * `edges_removed_with_evidence` counts only edges whose evidence_file
   * matched the reindexed path (a strict subset of total structural churn).
   * `cross_file_edges_staled` counts cross-file edges re-linked by name
   * after the surgical reindex (re-link attempts that succeeded at the
   * graph level; see the field doc below and the daemon `note`).
   * `file_missing` is true when the file did not exist at reindex time
   * (pure deletion scenario — nodes are removed, none inserted).
   */
  nodes_removed?: number;
  nodes_inserted?: number;
  edges_removed_with_evidence?: number;
  edges_inserted?: number;
  /** Count of cross-file edges successfully re-linked by name after surgical reindex. */
  cross_file_edges_staled?: number;
  /** Count of cross-file edges dropped because the file-side symbol no longer exists. */
  cross_file_edges_dropped?: number;
  file_missing?: boolean;
  /** Informational note from the daemon about telemetry semantics. */
  note?: string;
  /** Absolute path of the reindexed file (mode=fast only). */
  file?: string;
}

/** `inspect` method response body. */
export interface InspectResult {
  symbol: { name: string; kind: string; file: string; line: number } | null;
  edges_in: EdgeInfo[];
  edges_out: EdgeInfo[];
  freshness: { last_rebuild_at_ms: number };
}

export interface EdgeInfo {
  /** Present on `edges_in`: caller's name. */
  from?: string;
  /** Present on `edges_out`: callee's name. */
  to?: string;
  kind: string;
  origin: string;
  confidence: number;
  stale: number;
}

/** Minimal health check body. The Rust daemon returns `{ok, version, cached, symbols}`. */
export interface HealthResponse {
  ok: boolean;
  version: string;
  cached: boolean;
  symbols: number;
}

/** Per-symbol impact result inside an `impact_batch` response.
 * `null` signals the symbol did not resolve to any seed in the graph. */
export interface ImpactEntry {
  seeds: number;
  seeds_used: number;
  truncated: boolean;
  /** Transitive dependents reachable via incoming impact-bearing edges —
   * the symbols that would break if this one changed (callers, their
   * callers, …). File/doc container nodes are excluded. */
  nodes: number;
  edges: number;
  /** Sum of edge confidence weights across all traversed edges. */
  confidence_weighted: number;
}

/** `impact_batch` response body. Keys are the symbol names submitted
 * in the request; values are `ImpactEntry` or `null` (unknown symbol). */
export interface ImpactBatchResult {
  results: { [name: string]: ImpactEntry | null };
}

/** Single orphan node from `orphans` JSON output. `line` is 0-based. */
export interface OrphanReport {
  name: string;
  kind: string;
  file: string;
  line: number;
}

export interface OrphansResponse {
  count: number;
  orphans: OrphanReport[];
}

/** Single inconsistency from `inconsistencies` JSON output. The two
 * implicated nodes are reported symmetrically. */
export interface InconsistencyReport {
  category: "enum-mismatch" | "api-path" | "config-key" | string;
  shared_value: string;
  a: { name: string; file: string; line: number };
  b: { name: string; file: string; line: number };
}

export interface InconsistenciesResponse {
  count: number;
  reports: InconsistencyReport[];
}

/** Single entry in the `changed_files` array of a `diff` response.
 * Describes the git-based per-file impact for one changed file. */
export interface DiffFileImpact {
  file: string;
  seed_symbols: string[];
  reachable_count: number;
  confidence_weighted: number;
  /** Top 5 affected symbols by edge confidence, excluding the seeds themselves. */
  top_affected: Array<{ name: string; file: string; confidence: number }>;
  /** Keyed by 0-based line number (stringified for JSON). Populated by the
   * daemon path only; the non-daemon fallback omits it. Each entry records
   * which seed symbols are declared on that line and their combined
   * confidence-weighted impact score. */
  per_line_impact?: {
    [line: string]: { symbols: string[]; confidence_weighted: number };
  };
}

/** Full `diff` response body (daemon path — git-enriched).
 *
 * Produced by `dispatch_diff_with_git` in the Rust daemon when the
 * project root is available for git operations.  The non-daemon fallback
 * (`cached_diff`) returns the legacy `impacted_symbols` + `note` shape
 * instead — clients should check for the presence of `changed_files` to
 * distinguish the two paths.
 */
export interface DiffResult {
  base_ref: string;
  changed_files: DiffFileImpact[];
  total_reachable: number;
  total_confidence_weighted: number;
  inconsistencies_introduced: InconsistencyReport[];
  new_orphans: string[];
  git_operation_in_progress: boolean;
  /** Present on the non-daemon fallback path or when git is unavailable. */
  note?: string;
}
