//! Dispatches an IPC method to the appropriate in-process handler and
//! returns an `ipc::Response` (serialized to the client).
//!
//! Two entry points:
//! - `dispatch(method, params, project)` — build graph on demand (slow)
//! - `dispatch_cached(method, params, &graph)` — reuse a pre-built graph (fast)

use crate::commands::query::EdgeFilter;
use crate::global_opts::{ColorMode, OutputFormat};
use crate::ipc::Response;
use crate::render::{
    bfs_edges_aggregated, bfs_edges_filtered, decode_cursor, render_symbol, EdgeAtDepth,
};
use coregraph_core::SymbolNode;
use coregraph_extractor::build_graph;
use coregraph_graph::file_content_cache::FileContentCache;
use coregraph_graph::SymbolGraph;
use coregraph_query::{
    compute_impact, compute_risk, find_inconsistencies, find_orphans, query_symbol,
    InconsistencyCategory,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::OnceLock;

/// Maximum number of symbols accepted by a single `impact_batch` IPC call.
/// A request exceeding this is rejected with `ok: false` — in practice it
/// indicates a mis-configured client (e.g. passing a whole workspace symbol
/// list as the batch) rather than a legitimate batched query.
const MAX_SYMBOLS_PER_BATCH: usize = 64;

/// Maximum number of graph nodes that share one name and will be used as
/// impact seeds. Common identifiers like `new`, `init`, or `handle` can
/// easily match dozens of nodes; without a cap, a single IPC call would
/// fan out into that many depth-3 traversals synchronously. 16 is generous
/// enough for a reasonable overload set and small enough to keep response
/// time bounded.
const MAX_SEEDS_PER_NAME: usize = 16;

/// Process-wide LRU cache for file contents. Used by all LSP/MCP range
/// resolution calls so reading the same source file on repeated
/// `lsp.definition` requests hits RAM rather than disk.
fn range_cache() -> &'static FileContentCache {
    static CACHE: OnceLock<FileContentCache> = OnceLock::new();
    CACHE.get_or_init(|| FileContentCache::new(100))
}

/// Convert `(file, span_start, span_end)` into an LSP `range` JSON value.
///
/// Reads file content from the process-wide `range_cache`. Returns
/// `(0,0)-(0,0)` when the file cannot be read (e.g. synthetic paths in
/// tests that use non-existent fixture files).
fn resolve_range(file: &str, span_start: u32, span_end: u32) -> Value {
    match range_cache().get(Path::new(file)).ok().flatten() {
        Some(source) => {
            let (sl, sc) = coregraph_core::resolve_line_col(&source, span_start);
            let (el, ec) = coregraph_core::resolve_line_col(&source, span_end);
            json!({
                "start": {"line": sl, "character": sc},
                "end":   {"line": el, "character": ec},
            })
        }
        None => {
            // Emitted when the source file is missing or unreadable.
            // The IDE will show the navigation jump at line 0, col 0.
            // This is expected for synthetic fixture paths under test
            // and for files removed between indexing and query time.
            eprintln!(
                "resolve_range: file not found, falling back to (0,0): {}",
                file
            );
            json!({
                "start": {"line": 0, "character": 0},
                "end":   {"line": 0, "character": 0},
            })
        }
    }
}

fn parse_output_format(params: &Value) -> OutputFormat {
    match params.get("output_format").and_then(|v| v.as_str()) {
        Some("json") => OutputFormat::Json,
        Some("llm") => OutputFormat::Llm,
        _ => OutputFormat::Human,
    }
}

fn parse_langs(params: &Value) -> Vec<String> {
    params
        .get("lang")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn dispatch(method: &str, params: &Value, project: &Path) -> Response {
    match method {
        "query" => dispatch_query(params, project),
        "impact" => dispatch_impact(params, project),
        "orphans" => dispatch_orphans(params, project),
        "inconsistencies" => dispatch_inconsistencies(params, project),
        "stats" => dispatch_stats(params, project),
        // LSP/MCP methods route through the same one-shot build path
        // when no daemon is up. Each builds a fresh graph then forwards
        // to the cached handler so the response shape is identical
        // across the two execution modes.
        "lsp.definition" | "lsp.references" | "lsp.workspace_symbol" => {
            match build_graph(project) {
                Ok((g, _)) => dispatch_cached(method, params, &g),
                Err(e) => Response {
                    ok: false,
                    body: String::new(),
                    error: Some(e.to_string()),
                },
            }
        }
        "reindex" => reindex_mutable(params, project),
        "health" => Response {
            ok: true,
            body: format!(
                "{{\"ok\":true,\"version\":\"{}\"}}",
                env!("CARGO_PKG_VERSION")
            ),
            error: None,
        },
        other => Response {
            ok: false,
            body: String::new(),
            error: Some(format!("unknown method: {}", other)),
        },
    }
}

/// Variant that operates on a pre-built graph (used by the daemon after
/// initial indexing). No disk I/O, no parse.
pub fn dispatch_cached(method: &str, params: &Value, graph: &SymbolGraph) -> Response {
    match method {
        "query" => cached_query(params, graph),
        "impact" => cached_impact(params, graph),
        "orphans" => cached_orphans(params, graph, None),
        "inconsistencies" => cached_inconsistencies(params, graph),
        "stats" => cached_stats(params, graph),
        // LSP/MCP bridge methods. Each takes a symbol or query string
        // (the bridge has already done file/position → identifier
        // resolution on the client side) and returns a JSON body the
        // bridge wraps in protocol-specific envelopes.
        "lsp.definition" => cached_definition(params, graph),
        "lsp.references" => cached_references(params, graph),
        "lsp.workspace_symbol" => cached_workspace_symbol(params, graph),
        "inspect" => cached_inspect(params, graph),
        "reindex" => cached_reindex_unsupported(params, graph),
        "diff" => cached_diff(params, graph),
        "cross_lang" => cached_cross_lang(params, graph),
        "impact_batch" => cached_impact_batch(params, graph),
        "health" => Response {
            ok: true,
            body: format!(
                "{{\"ok\":true,\"version\":\"{}\",\"cached\":true,\"symbols\":{}}}",
                env!("CARGO_PKG_VERSION"),
                graph.node_count()
            ),
            error: None,
        },
        other => Response {
            ok: false,
            body: String::new(),
            error: Some(format!("unknown method: {}", other)),
        },
    }
}

/// Find every node whose `name` matches exactly. Returns a JSON array
/// of `{file, span_start, span_end}` so the LSP/MCP bridge can convert
/// to its own location format. The bridge owns line/column lookup
/// because that requires reading the source file the daemon doesn't
/// have a path to canonicalise.
fn cached_definition(params: &Value, g: &SymbolGraph) -> Response {
    let name = params.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() {
        return Response {
            ok: false,
            body: String::new(),
            error: Some("missing 'symbol'".into()),
        };
    }
    let locations: Vec<_> = g
        .nodes()
        .filter(|n| n.name == name)
        .map(|n| {
            let file = n.file.display().to_string();
            let range = resolve_range(&file, n.span_start, n.span_end);
            json!({
                "file": file,
                "span_start": n.span_start,
                "span_end": n.span_end,
                "kind": format!("{:?}", n.kind),
                "range": range,
            })
        })
        .collect();
    Response {
        ok: true,
        body: json!({ "locations": locations }).to_string(),
        error: None,
    }
}

/// Return every node connected to the named target via a semantic edge.
/// Used by `textDocument/references`.
///
/// We deliberately filter out `Resolves`, `Contains`, and `BelongsTo`:
/// `Resolves` accounts for the bulk of edges (name-resolution results)
/// and would drown the IDE's "Find References" popup in non-actionable
/// noise; `Contains` / `BelongsTo` are structural scaffolding (File →
/// symbol, symbol → module) with no code-reference meaning.
fn cached_references(params: &Value, g: &SymbolGraph) -> Response {
    use coregraph_core::EdgeKind;
    let name = params.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() {
        return Response {
            ok: false,
            body: String::new(),
            error: Some("missing 'symbol'".into()),
        };
    }
    let target_set: std::collections::HashSet<coregraph_core::SymbolId> =
        g.nodes().filter(|n| n.name == name).map(|n| n.id).collect();
    let is_semantic = |k: &EdgeKind| {
        !matches!(
            k,
            EdgeKind::Resolves | EdgeKind::Contains | EdgeKind::BelongsTo
        )
    };
    let locations: Vec<_> = g
        .edges()
        .filter(|e| is_semantic(&e.kind))
        .filter(|e| target_set.contains(&e.from) || target_set.contains(&e.to))
        .filter_map(|e| {
            let other_id = if target_set.contains(&e.from) {
                e.to
            } else {
                e.from
            };
            let other = g.get_node(other_id)?;
            let file = other.file.display().to_string();
            let range = resolve_range(&file, other.span_start, other.span_end);
            Some(json!({
                "file": file,
                "span_start": other.span_start,
                "span_end": other.span_end,
                "kind": format!("{:?}", other.kind),
                "range": range,
            }))
        })
        .collect();
    Response {
        ok: true,
        body: serde_json::json!({ "locations": locations }).to_string(),
        error: None,
    }
}

/// Substring search over symbol names, capped at 200 matches.
/// Drives `workspace/symbol` in LSP — the IDE's quick-symbol picker.
///
/// Returns `{"symbols": [...]}` where each entry carries `name`, `kind`,
/// `file`, and `range` at the top level.
///
/// The shape intentionally differs from `cached_definition`'s
/// `{"locations": [...]}` to match LSP's `WorkspaceSymbol` schema,
/// which requires `{name, kind, location}` per entry. The bridge in
/// `commands/lsp.rs::handle_workspace_symbol` wraps each symbol's
/// `file+range` into a `location` object before returning to the IDE.
fn cached_workspace_symbol(params: &Value, g: &SymbolGraph) -> Response {
    let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let symbols: Vec<_> = g
        .nodes()
        .filter(|n| query.is_empty() || n.name.contains(query))
        .take(200)
        .map(|n| {
            let file = n.file.display().to_string();
            let range = resolve_range(&file, n.span_start, n.span_end);
            json!({
                "name": n.name,
                "kind": format!("{:?}", n.kind),
                "file": file,
                "span_start": n.span_start,
                "span_end": n.span_end,
                "range": range,
            })
        })
        .collect();
    Response {
        ok: true,
        body: serde_json::json!({ "symbols": symbols }).to_string(),
        error: None,
    }
}

fn cached_query(params: &Value, g: &SymbolGraph) -> Response {
    // CLI sends `symbol`; older callers may still send `name`.
    let name = params
        .get("symbol")
        .or_else(|| params.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if name.is_empty() {
        return Response {
            ok: false,
            body: String::new(),
            error: Some("missing 'symbol'".into()),
        };
    }

    let page_size = params
        .get("page_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;
    let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
    let token_budget = params
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .unwrap_or(8000) as usize;
    let min_confidence = params
        .get("min_confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let include_stale = params
        .get("include_stale")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let output_format = parse_output_format(params);
    let langs = parse_langs(params);

    let start_page = params
        .get("cursor")
        .and_then(|v| v.as_str())
        .and_then(decode_cursor)
        .map(|c| c.page)
        .unwrap_or(0);

    let exact_hits = g.lookup_by_name(name, page_size * 4);
    let candidates: Vec<&coregraph_core::SymbolNode> = if !exact_hits.is_empty() {
        exact_hits
    } else {
        g.lookup_by_name_fuzzy(name, page_size * 4)
    };
    let matches: Vec<_> = candidates
        .into_iter()
        .filter(|n| crate::langfilter::match_langs(&langs, &n.file))
        .collect();

    if matches.is_empty() {
        return Response {
            ok: true,
            body: format!("No symbol found for '{}'", name),
            error: None,
        };
    }

    // Prefer an exported/Public definition as the center (mirrors the local
    // CLI path), so a test-local or module-private same-name node never hides
    // the public API symbol.
    let center_index = crate::render::pick_center_index(&matches);
    let center = matches[center_index];
    let depth = depth.max(1);

    // Default: precise single-symbol neighborhood (keeps distinct same-name
    // symbols separated). `aggregate` opts into unioning every same-name
    // definition's callers. Mirrors the local CLI path.
    let aggregate = params
        .get("aggregate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let (incoming_all, outgoing_all) = if aggregate {
        let center_ids: Vec<coregraph_core::SymbolId> = matches
            .iter()
            .filter(|n| n.name == center.name)
            .map(|n| n.id)
            .collect();
        bfs_edges_aggregated(g, &center_ids, depth, min_confidence, include_stale)
    } else {
        // Precise default = exact prior single-center traversal (no dedup).
        bfs_edges_filtered(g, center.id, depth, min_confidence, include_stale)
    };

    // Honor the direction + edge-kind filters forwarded by the thin client.
    // These were previously ignored on the daemon path, so `--edge-kind calls`
    // / `--direction incoming` had no effect whenever the daemon was running.
    let direction = params
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("both");
    let edge_filters: Vec<EdgeFilter> = params
        .get("edge_kind")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter_map(EdgeFilter::from_kebab)
                .collect()
        })
        .unwrap_or_default();
    let pass_edge = |e: &EdgeAtDepth| -> bool {
        edge_filters.is_empty() || edge_filters.iter().any(|f| f.matches(&e.edge.kind))
    };
    let incoming_all: Vec<EdgeAtDepth> = if direction == "outgoing" {
        vec![]
    } else {
        incoming_all.into_iter().filter(pass_edge).collect()
    };
    let outgoing_all: Vec<EdgeAtDepth> = if direction == "incoming" {
        vec![]
    } else {
        outgoing_all.into_iter().filter(pass_edge).collect()
    };

    let rendered = render_symbol(
        g,
        center,
        &incoming_all,
        &outgoing_all,
        output_format,
        ColorMode::Never,
        start_page,
        page_size,
        token_budget,
    );
    // Surface the other same-name definitions a precise query did not show
    // (suppressed when aggregating or for JSON consumers). Mirrors the local
    // CLI path so daemon-served output is never silently partial.
    let mut body = rendered.body;
    if !aggregate && !matches!(output_format, OutputFormat::Json) {
        if let Some(note) = crate::render::multi_def_note(&matches, center_index) {
            body.push('\n');
            body.push_str(&note);
        }
    }
    Response {
        ok: true,
        body,
        error: None,
    }
}

fn cached_impact(params: &Value, g: &SymbolGraph) -> Response {
    let name = params
        .get("symbol")
        .or_else(|| params.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    let transitive = params
        .get("transitive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let with_risk = params
        .get("risk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let output_format = parse_output_format(params);

    let Some(seed) = g.lookup_by_name(name, 1).first().copied().cloned() else {
        return Response {
            ok: true,
            body: format!("No symbol found for '{}'", name),
            error: None,
        };
    };

    let effective_depth = if transitive { depth } else { 1 };
    let impact = compute_impact(g, seed.id, effective_depth);
    let risk = if with_risk {
        Some(compute_risk(g, seed.id, effective_depth))
    } else {
        None
    };

    let body = match output_format {
        OutputFormat::Json => {
            let mut json = serde_json::json!({
                "seed": name,
                "reachable": impact.reachable.len(),
                "edges": impact.edges.len(),
                "depth": impact.depth_reached,
                "transitive": transitive,
            });
            if let Some(r) = &risk {
                json["risk"] = serde_json::json!({
                    "score": r.risk_score,
                    "level": format!("{:?}", r.risk_level),
                    "blast_radius": format!("{:?}", r.blast_radius),
                    "confidence_weighted_impact": r.confidence_weighted_impact,
                    "affected_tests": r.affected_tests.len(),
                });
            }
            json.to_string()
        }
        _ => {
            let header = format!(
                "Impact of '{}': {} reachable symbols, {} edges, depth {}{}",
                name,
                impact.reachable.len(),
                impact.edges.len(),
                impact.depth_reached,
                if transitive { " (transitive)" } else { "" },
            );
            let mut out = String::new();
            out.push_str(&header);
            out.push('\n');
            if let Some(r) = &risk {
                out.push_str(&format!(
                    "  Risk Score: {:.2} ({:?})\n",
                    r.risk_score, r.risk_level
                ));
                out.push_str(&format!("  Blast Radius: {:?}\n", r.blast_radius));
                out.push_str(&format!(
                    "  Confidence-Weighted Impact: {:.3}\n",
                    r.confidence_weighted_impact
                ));
                out.push_str(&format!("  Affected tests: {}\n", r.affected_tests.len()));
            }
            for n in impact.reachable.iter().take(50) {
                out.push_str(&format!(
                    "  {} [{:?}] — {}\n",
                    n.name,
                    n.kind,
                    n.file.display()
                ));
            }
            out
        }
    };

    Response {
        ok: true,
        body,
        error: None,
    }
}

/// Resolve a node's 0-based line number via the process-wide
/// `range_cache`. Returns 0 when the file cannot be read (synthetic
/// fixtures, deleted files). Used by Diagnostic-style IPC outputs
/// that need a Range to anchor the squiggle.
pub(crate) fn node_line(node: &coregraph_core::SymbolNode) -> u32 {
    match range_cache().get(&node.file).ok().flatten() {
        Some(src) => coregraph_core::resolve_line_col(&src, node.span_start).0,
        None => 0,
    }
}

/// Single source of truth for `orphans` output, shared by the daemon path
/// (`cached_orphans`) and the in-process CLI path (`commands::orphans::run`),
/// so `--output-format {human,llm,json}` renders identically regardless of
/// which path served the request. The two paths previously had drifted
/// formatters (flat-vs-rich JSON, sectioned-vs-tagged human text).
pub(crate) fn render_orphans(
    orphans: &[SymbolNode],
    format: OutputFormat,
    is_test: &dyn Fn(&SymbolNode) -> bool,
    is_api: &dyn Fn(&SymbolNode) -> bool,
    public_only: bool,
    has_library_signal: bool,
) -> String {
    let api_count = orphans.iter().filter(|n| is_api(n)).count();
    let test_count = orphans.iter().filter(|n| is_test(n)).count();
    let dead_count = orphans.len() - api_count - test_count;

    match format {
        OutputFormat::Json => serde_json::json!({
            "count": orphans.len(),
            "library_api_surface": api_count,
            "test_code": test_count,
            "likely_dead": dead_count,
            "orphans": orphans.iter().map(|n| serde_json::json!({
                "name": n.name,
                "kind": format!("{:?}", n.kind),
                "file": n.file.display().to_string(),
                // 0-based line of the symbol declaration. Falls back to
                // 0 when the file is unreadable (fixture / deleted).
                "line": node_line(n),
                "external_api": is_api(n),
                "is_test": is_test(n),
            })).collect::<Vec<_>>(),
        })
        .to_string(),
        OutputFormat::Llm => {
            let mut out = String::from("## Orphans\n");
            out.push_str(&format!("- count: {}\n", orphans.len()));
            out.push_str(&format!("- library_api_surface: {api_count}\n"));
            out.push_str(&format!("- test_code: {test_count}\n"));
            out.push_str(&format!("- likely_dead: {dead_count}\n"));
            if !orphans.is_empty() {
                out.push_str(
                    "\n| name | kind | file | external_api | is_test |\n|---|---|---|---|---|\n",
                );
                for n in orphans {
                    out.push_str(&format!(
                        "| {} | {:?} | {} | {} | {} |\n",
                        n.name,
                        n.kind,
                        n.file.display(),
                        is_api(n),
                        is_test(n)
                    ));
                }
            }
            out
        }
        OutputFormat::Human => {
            if orphans.is_empty() {
                return "Orphan symbols (0): no incoming or outgoing edges".to_string();
            }
            let mut out = format!(
                "Orphan symbols ({}): {} likely dead, {} library API surface, {} test code\n",
                orphans.len(),
                dead_count,
                api_count,
                test_count
            );
            for n in orphans.iter().take(200) {
                let tag = if is_test(n) {
                    " [test]"
                } else if is_api(n) {
                    " [library API]"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "  {} [{:?}] — {}{}\n",
                    n.name,
                    n.kind,
                    n.file.display(),
                    tag
                ));
            }
            // Public-only is the default; on a non-library project an
            // unreferenced public symbol is usually genuinely dead, but on a
            // library it is the external API. Surface that ambiguity once,
            // only when no manifest library signal disambiguated the buckets.
            if public_only && !has_library_signal {
                out.push_str(
                    "\nNote: these are public/exported symbols unreferenced within this repo. \
                     If this is a library, they are likely its external API, not dead code — \
                     run with --public-only=false to also include private symbols (higher-confidence dead code).\n",
                );
            }
            out
        }
    }
}

pub(crate) fn cached_orphans(params: &Value, g: &SymbolGraph, root: Option<&Path>) -> Response {
    let exclude_tests = params
        .get("exclude_tests")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Default false to preserve existing callers (e.g. MCP) that omit it; the
    // CLI `--public-only` fast-path sends true so the daemon result matches the
    // CLI local path instead of leaking private symbols.
    let public_only = params
        .get("public_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let output_format = parse_output_format(params);
    // Library classifier (manifest-derived) so a library package's public
    // orphans are labelled external API surface rather than dead code. `None`
    // root (callers without a project path) → no labelling.
    let classifier = root.map(coregraph_query::LibraryClassifier::from_project_root);
    // Mutually-exclusive buckets, test taking precedence over library (an inline
    // test in a library package is test code, not API surface). Relativise the
    // test-path check against the project root so a `tests` ANCESTOR outside the
    // project (e.g. a fixture under `…/tests/cache/proj`) is not matched.
    let is_test = |n: &SymbolNode| match root {
        Some(r) => coregraph_query::is_test_symbol_in(n, r),
        None => coregraph_query::is_test_symbol(n),
    };
    let is_api = |n: &SymbolNode| {
        !is_test(n)
            && classifier
                .as_ref()
                .is_some_and(|c| c.is_library_file(&n.file))
            && coregraph_query::is_public_symbol(n)
    };
    let orphans: Vec<_> = find_orphans(g)
        .into_iter()
        .filter(|n| !exclude_tests || !is_test(n))
        .filter(|n| !public_only || coregraph_query::is_public_symbol(n))
        .collect();
    let has_library_signal = classifier.as_ref().is_some_and(|c| c.has_signal());
    let body = render_orphans(
        &orphans,
        output_format,
        &is_test,
        &is_api,
        public_only,
        has_library_signal,
    );

    Response {
        ok: true,
        body,
        error: None,
    }
}

fn cached_inconsistencies(params: &Value, g: &SymbolGraph) -> Response {
    let category: Option<InconsistencyCategory> = params
        .get("category")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "enum-mismatch" => Some(InconsistencyCategory::EnumMismatch),
            "api-path" => Some(InconsistencyCategory::ApiPath),
            "config-key" => Some(InconsistencyCategory::ConfigKey),
            _ => None,
        });
    // When true, drop reports where either implicated node lives under a
    // test path (per `is_test_path`). Mirrors the existing `exclude_tests`
    // flag on `cached_orphans`. Default false so CLI callers keep the old
    // behaviour; the VSCode extension sets it to true to reduce Problems
    // panel noise from fixture files.
    let exclude_tests = params
        .get("exclude_tests")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let output_format = parse_output_format(params);
    let reports: Vec<_> = find_inconsistencies(g)
        .into_iter()
        .filter(|r| category.is_none_or(|c| r.category == c))
        .filter(|r| {
            if !exclude_tests {
                return true;
            }
            !coregraph_query::is_test_path(&r.node_a.file)
                && !coregraph_query::is_test_path(&r.node_b.file)
        })
        .collect();

    let body = match output_format {
        OutputFormat::Json => serde_json::json!({
            "count": reports.len(),
            "reports": reports.iter().map(|r| serde_json::json!({
                "category": format!("{:?}", r.category),
                "shared_value": r.shared_value,
                "a": {
                    "name": r.node_a.name,
                    "file": r.node_a.file.display().to_string(),
                    "line": node_line(&r.node_a),
                },
                "b": {
                    "name": r.node_b.name,
                    "file": r.node_b.file.display().to_string(),
                    "line": node_line(&r.node_b),
                },
            })).collect::<Vec<_>>(),
        })
        .to_string(),
        _ => {
            if reports.is_empty() {
                "No inconsistencies detected.".to_string()
            } else {
                let mut out = format!("Inconsistencies ({}):\n", reports.len());
                for r in reports.iter().take(200) {
                    match r.category {
                        InconsistencyCategory::EnumMismatch => {
                            out.push_str(&format!(
                                "  [enum-mismatch] {} vs {}\n    - {} ({})\n    - {} ({})\n",
                                r.node_a.name,
                                r.node_b.name,
                                r.node_a.name,
                                r.node_a.file.display(),
                                r.node_b.name,
                                r.node_b.file.display(),
                            ));
                        }
                        InconsistencyCategory::ApiPath => {
                            out.push_str(&format!(
                                "  [api-path] {}\n    - {}\n    - {}\n",
                                r.shared_value,
                                r.node_a.file.display(),
                                r.node_b.file.display(),
                            ));
                        }
                        InconsistencyCategory::ConfigKey => {
                            out.push_str(&format!(
                                "  [config-key] {}\n    - {} ({})\n",
                                r.shared_value,
                                r.node_a.name,
                                r.node_a.file.display(),
                            ));
                        }
                    }
                }
                out
            }
        }
    };

    Response {
        ok: true,
        body,
        error: None,
    }
}

fn cached_stats(params: &Value, g: &SymbolGraph) -> Response {
    let output_format = parse_output_format(params);
    match output_format {
        OutputFormat::Json => Response {
            ok: true,
            body: serde_json::json!({
                "symbols": g.node_count(),
                "edges": g.edge_count(),
            })
            .to_string(),
            error: None,
        },
        _ => Response {
            ok: true,
            body: format!("symbols: {}\nedges: {}", g.node_count(), g.edge_count()),
            error: None,
        },
    }
}

/// Validates the `mode` parameter for a reindex request but always refuses to
/// execute: `dispatch_cached` operates on a shared `&SymbolGraph` which is
/// immutable, so an actual reindex (which requires rebuilding or patching the
/// graph) cannot happen here.  The daemon routes reindex requests through
/// `dispatch`, which owns the mutable project state.
fn cached_reindex_unsupported(params: &Value, _graph: &SymbolGraph) -> Response {
    let mode = params
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("full");
    if mode != "fast" && mode != "full" {
        return Response {
            ok: false,
            body: String::new(),
            error: Some(format!("invalid mode '{mode}' (expected 'fast'|'full')")),
        };
    }
    Response {
        ok: false,
        body: String::new(),
        error: Some(
            "reindex requires mutable graph state; send reindex requests \
             directly to the daemon (not the cached read path)"
                .into(),
        ),
    }
}

/// Uncached reindex handler. The `dispatch` entry point has no prior graph
/// state, so `mode=full` does a fresh `build_graph(project)` and reports
/// stats; `mode=fast` has nothing to incrementally update and returns an
/// honest error directing the caller to the daemon (which owns the mutable
/// graph). Per CLAUDE.md: no stub `ok: true` for the fast path in this
/// context.
fn reindex_mutable(params: &Value, project: &Path) -> Response {
    let started = std::time::Instant::now();
    let mode = params
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("full");
    let file = params
        .get("file")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from);

    match (mode, file) {
        ("full", _) => match build_graph(project) {
            Ok((g, _)) => Response {
                ok: true,
                body: serde_json::json!({
                    "reindexed": true,
                    "mode": "full",
                    "elapsed_ms": started.elapsed().as_millis() as u64,
                    "node_count": g.node_count(),
                    "edge_count": g.edge_count(),
                })
                .to_string(),
                error: None,
            },
            Err(e) => Response {
                ok: false,
                body: String::new(),
                error: Some(format!("full reindex failed: {e}")),
            },
        },
        ("fast", Some(_path)) => Response {
            ok: false,
            body: String::new(),
            error: Some(
                "fast mode reindex requires a running daemon that owns the mutable graph; \
                 start the server (`coregraph server start`) or use mode=full for a \
                 one-shot rebuild"
                    .into(),
            ),
        },
        ("fast", None) => Response {
            ok: false,
            body: String::new(),
            error: Some("fast mode requires a 'file' parameter".into()),
        },
        (m, _) => Response {
            ok: false,
            body: String::new(),
            error: Some(format!("invalid mode '{m}' (expected 'fast'|'full')")),
        },
    }
}

/// Daemon-side mutable reindex handler. Called when the running daemon
/// holds a write guard on the graph — that is the only context where
/// graph mutation is legal.
///
/// mode=full: rebuild the graph from scratch using the same canonical
///   pipeline the daemon uses at startup (`load_project_graph_only`).
///   Replaces the graph contents in place and resets freshness metadata
///   via `mark_full_rebuild`.
///
/// mode=fast with file: surgical single-file reindex. Removes the
///   file's existing nodes (which drops incident edges), re-extracts
///   directly into the live graph, and bumps `stale_evidence_count`
///   on surviving cross-file edges incident to the newly-inserted nodes.
pub fn dispatch_reindex_mutable(
    params: &Value,
    graph: &mut SymbolGraph,
    project: &Path,
) -> Response {
    let started = std::time::Instant::now();
    let mode = params
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("full");
    let file = params
        .get("file")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from);

    match (mode, file) {
        ("full", _) => match crate::graph_loader::load_project_graph_only(project) {
            Ok(new_graph) => {
                *graph = new_graph;
                graph.mark_full_rebuild();
                Response {
                    ok: true,
                    body: serde_json::json!({
                        "reindexed": true,
                        "mode": "full",
                        "elapsed_ms": started.elapsed().as_millis() as u64,
                        "node_count": graph.node_count(),
                        "edge_count": graph.edge_count(),
                    })
                    .to_string(),
                    error: None,
                }
            }
            Err(e) => Response {
                ok: false,
                body: String::new(),
                error: Some(format!("full reindex failed: {e}")),
            },
        },
        ("fast", Some(path)) => reindex_file_in_place(graph, &path, started),
        ("fast", None) => Response {
            ok: false,
            body: String::new(),
            error: Some("fast mode requires a 'file' parameter".into()),
        },
        (m, _) => Response {
            ok: false,
            body: String::new(),
            error: Some(format!("invalid mode '{m}' (expected 'fast'|'full')")),
        },
    }
}

/// Fast single-file reindex: remove the file's old nodes (which drops
/// all incident edges via petgraph), then re-extract into the live graph.
///
/// Telemetry note: `edges_removed_with_evidence` reports only edges whose
/// `evidence_file` matched the reindexed path. `remove_node` additionally
/// drops edges incident to removed nodes that were evidenced in OTHER
/// files — those are not counted here by design, since the field is about
/// evidence attribution, not total structural churn.
///
/// Cross-file re-link (0.5c): before removing old nodes we snapshot every
/// cross-file edge (i.e. `evidence_file != path`) that is incident to one of
/// the file's symbols. After re-extraction we attempt to re-attach each
/// snapshot to the newly-inserted node whose `qualified_name` (or bare `name`
/// as independent fallback) matches the old file-side endpoint. Successfully
/// re-linked edges receive `stale_evidence_count + 1` so downstream confidence
/// decay reflects the reduced certainty. Edges whose file-side symbol was
/// deleted or renamed during the edit are counted in `cross_file_edges_dropped`
/// and lost until the next full rebuild.
///
/// Counter semantic note: `cross_file_edges_staled` counts re-link ATTEMPTS
/// that succeeded at the graph level. `insert_edge` dedups on
/// `(from, to, kind_discriminant)` and returns `true` for a no-op when an
/// identical triple already exists; in that case the attempt is counted even
/// though only one physical edge remains. Two captured edges with the same
/// endpoint triple but different `evidence_file` both contribute to the
/// counter, even though the graph holds a single edge. This is intentional
/// for Phase 0.5c — a future phase can tighten if dedup overcount becomes
/// measurable.
fn reindex_file_in_place(
    graph: &mut SymbolGraph,
    path: &Path,
    started: std::time::Instant,
) -> Response {
    use coregraph_core::DirectEdge;
    use std::collections::HashSet;

    // Read source; if the file is gone, treat as deletion (remove only).
    let source = std::fs::read_to_string(path).ok();

    // Snapshot the file's existing node ids and evidence-attributed
    // edge count for telemetry. `edges_removed_with_evidence` is a
    // strict subset of total edges dropped by the subsequent
    // `remove_node` calls — see the fn docstring.
    let old_node_ids: Vec<_> = graph
        .nodes()
        .filter(|n| n.file.as_ref() == path)
        .map(|n| n.id)
        .collect();
    let nodes_removed = old_node_ids.len();
    let edges_removed_with_evidence = graph.edges_for_file(path).count();

    // === CAPTURE phase ===
    //
    // Snapshot every cross-file edge incident to the file's nodes so we can
    // attempt to re-link them after re-extraction. We only capture edges
    // whose evidence_file differs from the reindexed path; same-file evidence
    // edges are simply re-emitted by the extractor.
    struct CapturedEdge {
        /// Qualified name of the OLD file-side node (may be empty when the
        /// extractor did not populate qualified_name). Used for a primary
        /// lookup against `lookup_by_qualified_name`, which is keyed by
        /// qualified_name — NOT the bare name.
        file_qname: String,
        /// Bare name of the OLD file-side node (always populated — we skip
        /// capture when both qname and name are empty). Used for a secondary,
        /// independent lookup against `lookup_by_name`, which is keyed by
        /// the bare `name`. Passing a qualified name to `lookup_by_name`
        /// would never match, so the two keys must be captured separately.
        file_name: String,
        /// Direction from the perspective of the file-side node.
        incoming: bool,
        /// The endpoint OUTSIDE the reindexed file (the one we must preserve).
        other_id: coregraph_core::SymbolId,
        kind: coregraph_core::edge::EdgeKind,
        origin: coregraph_core::edge::AnalysisOrigin,
        confidence: coregraph_core::edge::Confidence,
        evidence_file: std::path::PathBuf,
        stale_evidence_count: u32,
        created_at_epoch: u64,
    }

    let old_ids_set: HashSet<_> = old_node_ids.iter().copied().collect();
    let mut captured: Vec<CapturedEdge> = Vec::new();

    for edge in graph.edges() {
        // Skip same-file evidence edges — the extractor will re-emit them.
        if edge.evidence_file.as_ref() == path {
            continue;
        }
        let from_in_file = old_ids_set.contains(&edge.from);
        let to_in_file = old_ids_set.contains(&edge.to);
        // Skip edges that don't touch the file at all.
        if !from_in_file && !to_in_file {
            continue;
        }
        // Edges where BOTH endpoints are in the file are intra-file, not
        // cross-file — even if their evidence_file is elsewhere. Skip them;
        // re-extraction will rebuild intra-file edges naturally.
        if from_in_file && to_in_file {
            continue;
        }

        let (file_id, other_id, incoming) = if to_in_file {
            (edge.to, edge.from, true)
        } else {
            (edge.from, edge.to, false)
        };

        // Capture BOTH qualified_name and bare name independently so the
        // re-link phase can try `lookup_by_qualified_name` AND `lookup_by_name`
        // without cross-pollution. The two indexes use different keys: passing
        // `"mymod::callee"` to `lookup_by_name` would never match, so a single
        // captured key would render one fallback dead.
        let node = graph.get_node(file_id);
        let file_qname = node.map(|n| n.qualified_name.clone()).unwrap_or_default();
        let file_name = node.map(|n| n.name.clone()).unwrap_or_default();
        // At minimum we need a bare name; without it neither lookup can match.
        if file_name.is_empty() && file_qname.is_empty() {
            continue;
        }

        captured.push(CapturedEdge {
            file_qname,
            file_name,
            incoming,
            other_id,
            kind: edge.kind.clone(),
            origin: edge.origin,
            confidence: edge.confidence,
            evidence_file: edge.evidence_file.to_path_buf(),
            stale_evidence_count: edge.stale_evidence_count,
            created_at_epoch: edge.created_at_epoch,
        });
    }

    // === REMOVE phase ===
    for id in &old_node_ids {
        graph.remove_node(*id);
    }

    // === RE-EXTRACT phase ===
    // If the file still exists, re-extract directly into the live graph.
    // The extractor allocates fresh SymbolIds via insert_node.
    let node_count_before = graph.node_count();
    let edge_count_before = graph.edge_count();
    if let Some(ref src) = source {
        for extractor in coregraph_extractor::all_extractors() {
            if coregraph_extractor::scanner::extension_matches(path, extractor.file_extensions()) {
                // Extraction errors surface as empty insertion; we track
                // progress via node/edge count deltas.
                let _ = extractor.extract(path, src, graph);
                break;
            }
        }
    }
    let nodes_inserted = graph.node_count().saturating_sub(node_count_before);
    let edges_inserted = graph.edge_count().saturating_sub(edge_count_before);

    // === RE-LINK phase ===
    //
    // For each captured cross-file edge, find the new node in the reindexed
    // file whose qualified_name (or name fallback) matches the old file-side
    // endpoint. If found, reconstruct the edge with stale_evidence_count + 1
    // to signal reduced confidence. If not found, the symbol was deleted or
    // renamed — count it as dropped.
    let mut cross_file_edges_staled: usize = 0;
    let mut cross_file_edges_dropped: usize = 0;

    for cap in captured {
        // Primary: lookup by qualified_name (only when we actually captured
        // one). Passing an empty string into the index would trigger a
        // pointless `get("")` — skip the call entirely.
        let by_qname = if !cap.file_qname.is_empty() {
            graph
                .lookup_by_qualified_name(&cap.file_qname, 4)
                .into_iter()
                .find(|n| n.file.as_ref() == path)
        } else {
            None
        };
        // Fallback: lookup by bare name against the name index. Critically,
        // this uses `cap.file_name` (NOT `cap.file_qname`) — the two indexes
        // are keyed by different strings.
        let new_file_node = by_qname.or_else(|| {
            if cap.file_name.is_empty() {
                None
            } else {
                graph
                    .lookup_by_name(&cap.file_name, 4)
                    .into_iter()
                    .find(|n| n.file.as_ref() == path)
            }
        });

        let Some(new_node) = new_file_node else {
            // Symbol was deleted or renamed — edge is unrecoverable until
            // the next full rebuild.
            cross_file_edges_dropped += 1;
            continue;
        };
        let new_node_id = new_node.id;

        // Verify the other endpoint is still alive in the graph.
        if graph.get_node(cap.other_id).is_none() {
            cross_file_edges_dropped += 1;
            continue;
        }

        let (from, to) = if cap.incoming {
            (cap.other_id, new_node_id)
        } else {
            (new_node_id, cap.other_id)
        };

        let new_edge = DirectEdge::new(
            from,
            to,
            cap.kind,
            cap.origin,
            cap.confidence,
            cap.evidence_file,
        )
        .with_epoch(cap.created_at_epoch)
        .with_stale_evidence_count(cap.stale_evidence_count.saturating_add(1));

        // insert_edge returns false only if an endpoint id is unknown.
        // A dedup no-op returns true (edge already present, nothing to
        // count) — we still credit it as a successful re-link because the
        // edge is alive in the graph.
        // KNOWN: insert_edge returning true includes dedup no-ops. When two
        // captured edges share (from, to, kind) but differ in evidence_file,
        // both count toward cross_file_edges_staled even though only one
        // physical edge exists. Acceptable for Phase 0.5c; track in
        // docs/graph-model.md §6.3 if it becomes measurable.
        if graph.insert_edge(new_edge) {
            cross_file_edges_staled += 1;
        } else {
            // Endpoint vanished between the look-up and insert (shouldn't
            // happen in a single-threaded daemon, but be honest if it does).
            cross_file_edges_dropped += 1;
        }
    }

    graph.mark_fast_update(path);

    Response {
        ok: true,
        body: serde_json::json!({
            "reindexed": true,
            "mode": "fast",
            "file": path.display().to_string(),
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "nodes_removed": nodes_removed,
            "nodes_inserted": nodes_inserted,
            "edges_removed_with_evidence": edges_removed_with_evidence,
            "edges_inserted": edges_inserted,
            "cross_file_edges_staled": cross_file_edges_staled,
            "cross_file_edges_dropped": cross_file_edges_dropped,
            "file_missing": source.is_none(),
            "note": "cross_file_edges_staled counts re-link ATTEMPTS that succeeded at the graph level; dedup no-ops count once per attempt, not once per underlying edge. cross_file_edges_dropped = edges whose file-side endpoint no longer exists post-reindex.",
        })
        .to_string(),
        error: None,
    }
}

/// Return the symbol at the given file/line/column position together with its
/// incoming and outgoing edges and graph freshness metadata.
///
/// File matching uses `Path::ends_with` so a bare filename like `"foo.rs"`
/// matches a node whose `file` is `/proj/foo.rs`. Line resolution requires
/// the source file to be readable via the process-wide `range_cache`; when
/// the file is unavailable (e.g. synthetic fixture paths in tests) the match
/// falls through to the no-symbol branch, which still returns `ok: true` with
/// `symbol: null` and empty edge arrays so callers can always parse the shape.
fn cached_inspect(params: &Value, g: &SymbolGraph) -> Response {
    let file = params.get("file").and_then(|v| v.as_str()).unwrap_or("");
    let line = params.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    // column is accepted for API forward-compat (future multi-symbol-per-line
    // disambiguation) but not used for line-based matching today.
    let _column = params.get("column").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let file_path = Path::new(file);

    // Read source once for line-to-byte-offset resolution.
    let source = range_cache().get(file_path).ok().flatten();

    // Match all nodes whose file ends with the given path segment and whose
    // span_start resolves to the requested line number, then pick the most
    // specific one (narrowest span). This avoids `File`-kind symbols from
    // winning over a contained function / method that declares at line 0 of
    // the file — the file itself starts at offset 0 too, so both match.
    // Narrower span = more specific = better UX for Hover / StatusBar.
    use coregraph_core::SymbolKind;
    let candidates: Vec<_> = g
        .nodes()
        .filter(|n| {
            if !n.file.ends_with(file) {
                return false;
            }
            match source {
                Some(ref src) => {
                    let (node_line, _) = coregraph_core::resolve_line_col(src, n.span_start);
                    node_line == line
                }
                None => false,
            }
        })
        .collect();
    // Sort order: non-File first (File is a structural container, rarely what
    // the user is asking about), then ascending span width (narrow = specific).
    let symbol = candidates.into_iter().min_by_key(|n| {
        let is_file = matches!(n.kind, SymbolKind::File);
        let span = n.span_end.saturating_sub(n.span_start);
        // (is_file as u32, span) — File-kind sorts last regardless of span.
        (is_file as u32, span)
    });

    let freshness_ms = g.last_full_rebuild_at().elapsed().as_millis() as u64;

    let body = match symbol {
        Some(node) => {
            let (node_line, _col) = source
                .as_ref()
                .map(|src| coregraph_core::resolve_line_col(src, node.span_start))
                .unwrap_or((0, 0));

            let edges_in: Vec<_> = g
                .edges()
                .filter(|e| e.to == node.id)
                .map(|e| {
                    json!({
                        "from": g.get_node(e.from).map(|n| n.name.clone()),
                        "kind": format!("{:?}", e.kind),
                        "origin": format!("{:?}", e.origin),
                        "confidence": e.current_confidence(),
                        "stale": e.stale_evidence_count,
                    })
                })
                .collect();

            let edges_out: Vec<_> = g
                .edges()
                .filter(|e| e.from == node.id)
                .map(|e| {
                    json!({
                        "to": g.get_node(e.to).map(|n| n.name.clone()),
                        "kind": format!("{:?}", e.kind),
                        "origin": format!("{:?}", e.origin),
                        "confidence": e.current_confidence(),
                        "stale": e.stale_evidence_count,
                    })
                })
                .collect();

            json!({
                "symbol": {
                    "name": node.name,
                    "kind": format!("{:?}", node.kind),
                    "file": node.file.display().to_string(),
                    "line": node_line,
                },
                "edges_in": edges_in,
                "edges_out": edges_out,
                "freshness": {
                    "last_rebuild_at_ms": freshness_ms,
                },
            })
        }
        None => json!({
            "symbol": null,
            "edges_in": [],
            "edges_out": [],
            "freshness": {
                "last_rebuild_at_ms": freshness_ms,
            },
        }),
    };

    Response {
        ok: true,
        body: body.to_string(),
        error: None,
    }
}

/// Return the best-available "diff" signal for the current graph state.
///
/// Phase 0 approximation: no git integration is available yet, so we cannot
/// compute a true base_ref delta. Instead, this handler derives honest signals
/// from graph state:
///
/// - `impacted_symbols`: symbols whose outgoing edges have `stale_evidence_count > 0`,
///   meaning a recent fast-path reindex marked them as potentially impacted.
/// - `inconsistencies_introduced`: all current inconsistencies from
///   `find_inconsistencies`. "Introduced vs baseline" requires Phase 1 git diff.
fn cached_diff(_params: &Value, g: &SymbolGraph) -> Response {
    // Collect `from` nodes of edges that have accumulated stale evidence.
    // Deduplication via a set on `SymbolId` avoids listing the same symbol
    // multiple times when several of its outgoing edges are stale. Collect
    // as (file, name) tuples so the subsequent sort is on stable string
    // data (not on `SymbolId` order, which depends on node insertion).
    let mut seen = std::collections::HashSet::new();
    let mut impacted: Vec<(String, String)> = g
        .edges()
        .filter(|e| e.stale_evidence_count > 0)
        .filter_map(|e| g.get_node(e.from))
        .filter(|n| seen.insert(n.id))
        .map(|n| (n.file.display().to_string(), n.name.clone()))
        .collect();
    // Sort by (file, name) so repeated calls on an unchanged graph return
    // byte-identical bodies. `HashSet<SymbolId>` uses a randomized hasher
    // and `g.edges()` is petgraph insertion order; without an explicit sort
    // the output order is observable-but-unstable, which breaks snapshot
    // tests and makes client polling noisy.
    impacted.sort();
    let impacted_symbols: Vec<_> = impacted
        .into_iter()
        .map(|(file, name)| serde_json::json!({ "name": name, "file": file }))
        .collect();

    let mut inc_sorted: Vec<_> = find_inconsistencies(g);
    // Sort by (category, shared_value, node_a.name, node_b.name). Derive
    // ordering from the `Debug` rendering of category — the enum is small
    // and derivation via `format!` is cheap compared to the surrounding
    // graph traversal.
    inc_sorted.sort_by(|a, b| {
        let ka = (
            format!("{:?}", a.category),
            a.shared_value.clone(),
            a.node_a.name.clone(),
            a.node_b.name.clone(),
        );
        let kb = (
            format!("{:?}", b.category),
            b.shared_value.clone(),
            b.node_a.name.clone(),
            b.node_b.name.clone(),
        );
        ka.cmp(&kb)
    });
    let inconsistencies_introduced: Vec<_> = inc_sorted
        .iter()
        .map(|inc| {
            serde_json::json!({
                "category": format!("{:?}", inc.category),
                "node_a": inc.node_a.name,
                "node_b": inc.node_b.name,
                "shared_value": inc.shared_value,
            })
        })
        .collect();

    let body = serde_json::json!({
        "impacted_symbols": impacted_symbols,
        "inconsistencies_introduced": inconsistencies_introduced,
        "note": "Phase 0 fallback (non-daemon path): impacted_symbols derived from stale_evidence_count on edges. For git-enriched per-file impact, use the daemon path which routes 'diff' to dispatch_diff_with_git.",
    });
    Response {
        ok: true,
        body: body.to_string(),
        error: None,
    }
}

/// Git-enriched diff handler. Computes real per-file impact analysis for
/// the working tree vs `base_ref` (default `HEAD`) using the git CLI and
/// the symbol graph. This is the daemon path — it requires a project root
/// for git operations. The `cached_diff` fallback is used when no project
/// context is available.
///
/// Response shape: `{base_ref, changed_files, total_reachable,
/// total_confidence_weighted, inconsistencies_introduced, new_orphans,
/// git_operation_in_progress}`. See docs/cli.md §diff for field semantics.
///
/// Complexity note: O(F × S × depth³ × E) for impact traversal plus
/// O(O × 1) for orphan filtering (O = total orphans). Acceptable for
/// typical PRs (< 20 changed files, < 100 seeds).
pub fn dispatch_diff_with_git(params: &Value, g: &SymbolGraph, project: &Path) -> Response {
    use coregraph_watcher::git_diff::GitDiffStrategy;
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::path::PathBuf;

    let base_ref = params
        .get("base_ref")
        .and_then(|v| v.as_str())
        .unwrap_or("HEAD");

    // Detect merge/rebase/cherry-pick early so the caller knows the
    // diff may be noisy (many changed files, incomplete index).
    let git_op_in_progress = GitDiffStrategy::detect_git_operation(project);

    // Graceful degradation: git failure (not a repo, bad rev, git
    // unavailable) returns ok:true with empty arrays rather than an
    // error. Upstream consumers (VSCode extension) treat an empty diff
    // as "nothing changed" rather than an error state.
    let changed: Vec<PathBuf> =
        match GitDiffStrategy::changed_files_between(project, base_ref, "HEAD") {
            Ok(paths) => paths,
            Err(_) => {
                let body = json!({
                    "base_ref": base_ref,
                    "changed_files": [],
                    "total_reachable": 0,
                    "total_confidence_weighted": 0.0,
                    "inconsistencies_introduced": [],
                    "new_orphans": [],
                    "git_operation_in_progress": git_op_in_progress,
                    "note": "git unavailable or not a repository — returning empty diff",
                });
                return Response {
                    ok: true,
                    body: body.to_string(),
                    error: None,
                };
            }
        };

    let changed_set: HashSet<PathBuf> = changed.iter().cloned().collect();

    // Per-file impact accumulation.
    let mut per_file: Vec<serde_json::Value> = Vec::new();
    // Global dedup: a node reachable from multiple changed files is
    // counted once in total_reachable.
    let mut total_reachable: HashSet<coregraph_core::SymbolId> = HashSet::new();
    let mut total_conf = 0.0_f64;

    for file in &changed {
        // Collect seeds: all graph nodes whose file matches this changed path.
        let seeds: Vec<(coregraph_core::SymbolId, String)> = g
            .nodes_in_file(file)
            .map(|n| (n.id, n.name.clone()))
            .collect();

        if seeds.is_empty() {
            // File appears in the git diff but has no nodes in the graph.
            // Likely a new/deleted non-source file (config, docs, assets).
            per_file.push(json!({
                "file": file.display().to_string(),
                "seed_symbols": Vec::<String>::new(),
                "reachable_count": 0,
                "confidence_weighted": 0.0,
                "top_affected": Vec::<serde_json::Value>::new(),
            }));
            continue;
        }

        let seed_names: Vec<String> = seeds.iter().map(|(_, n)| n.clone()).collect();

        // Aggregate impact across all seeds in this file.
        // node_score: max confidence of any edge connecting to that node,
        // across all seeds. Used for top-5 ranking.
        let mut reached: HashSet<coregraph_core::SymbolId> = HashSet::new();
        let mut conf_sum = 0.0_f64;
        let mut node_score: HashMap<coregraph_core::SymbolId, f64> = HashMap::new();

        // per_line_impact: keyed by 0-based declaration line of each seed symbol,
        // maps to (symbol names, sum of edge confidences for that seed).
        // Multiple symbols on the same line merge into a single entry.
        let mut per_line: BTreeMap<u32, (Vec<String>, f64)> = BTreeMap::new();

        for (seed_id, seed_name) in &seeds {
            let res = compute_impact(g, *seed_id, 3);
            for node in &res.reachable {
                reached.insert(node.id);
            }
            // Sum confidence across edges touched by this seed for the line entry.
            let seed_conf: f64 = res.edges.iter().map(|e| e.current_confidence()).sum();
            for e in &res.edges {
                let c = e.current_confidence();
                conf_sum += c;
                // Score each endpoint of the edge (bidirectional traversal
                // means either end may be the "reached" node). Using max
                // so repeated edges from different seeds don't inflate.
                let score_from = node_score.entry(e.from).or_insert(0.0);
                *score_from = score_from.max(c);
                let score_to = node_score.entry(e.to).or_insert(0.0);
                *score_to = score_to.max(c);
            }
            // Emit a per-line entry at the seed's declaration line.
            if let Some(node) = g.get_node(*seed_id) {
                let line = node_line(node);
                let entry = per_line.entry(line).or_insert_with(|| (Vec::new(), 0.0));
                entry.0.push(seed_name.clone());
                entry.1 += seed_conf;
            }
        }

        // Top 5 by confidence, excluding seed nodes themselves.
        let seed_ids: HashSet<coregraph_core::SymbolId> = seeds.iter().map(|(id, _)| *id).collect();
        let mut top: Vec<(coregraph_core::SymbolId, f64)> = node_score
            .into_iter()
            .filter(|(id, _)| !seed_ids.contains(id))
            .collect();
        top.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0 .0.cmp(&b.0 .0)) // stable secondary sort by raw id u64
        });
        top.truncate(5);

        let top_affected: Vec<serde_json::Value> = top
            .into_iter()
            .filter_map(|(id, conf)| {
                let n = g.get_node(id)?;
                Some(json!({
                    "name": n.name,
                    "file": n.file.display().to_string(),
                    "confidence": conf,
                }))
            })
            .collect();

        // Serialize per_line into a JSON object: {"<line>": {symbols, confidence_weighted}}.
        let per_line_impact: serde_json::Value = per_line
            .into_iter()
            .map(|(line, (symbols, conf))| {
                (
                    line.to_string(),
                    json!({
                        "symbols": symbols,
                        "confidence_weighted": conf,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into();

        per_file.push(json!({
            "file": file.display().to_string(),
            "seed_symbols": seed_names,
            "reachable_count": reached.len(),
            "confidence_weighted": conf_sum,
            "top_affected": top_affected,
            "per_line_impact": per_line_impact,
        }));

        total_reachable.extend(reached);
        total_conf += conf_sum;
    }

    // Inconsistencies introduced: filter to those where either side's
    // file is in the changed set. Approximates "introduced by this diff"
    // since we cannot compare against a baseline snapshot (Phase 2.0 MVP).
    let mut inc_raw: Vec<_> = find_inconsistencies(g)
        .into_iter()
        .filter(|r| {
            changed_set.contains(r.node_a.file.as_ref())
                || changed_set.contains(r.node_b.file.as_ref())
        })
        .map(|r| {
            (
                format!("{:?}", r.category),
                r.shared_value.clone(),
                r.node_a.name.clone(),
                r.node_b.name.clone(),
                r,
            )
        })
        .collect();
    // Sort for deterministic output across repeated calls.
    inc_raw.sort_by(|a, b| (&a.0, &a.1, &a.2, &a.3).cmp(&(&b.0, &b.1, &b.2, &b.3)));

    let inconsistencies_introduced: Vec<serde_json::Value> = inc_raw
        .into_iter()
        .map(|(_, _, _, _, r)| {
            json!({
                "category": format!("{:?}", r.category),
                "shared_value": r.shared_value,
                "a": {
                    "name": r.node_a.name,
                    "file": r.node_a.file.display().to_string(),
                    "line": node_line(&r.node_a),
                },
                "b": {
                    "name": r.node_b.name,
                    "file": r.node_b.file.display().to_string(),
                    "line": node_line(&r.node_b),
                },
            })
        })
        .collect();

    // New orphans: symbols in changed files that have zero semantic edges.
    // Uses find_orphans(g) filtered to changed files — DRY, and consistent
    // with the existing orphan definition (ignores Contains/BelongsTo).
    let mut new_orphans: Vec<String> = find_orphans(g)
        .into_iter()
        .filter(|n| changed_set.contains(n.file.as_ref()))
        .map(|n| n.name)
        .collect();
    new_orphans.sort();
    new_orphans.dedup();

    let body = json!({
        "base_ref": base_ref,
        "changed_files": per_file,
        "total_reachable": total_reachable.len(),
        "total_confidence_weighted": total_conf,
        "inconsistencies_introduced": inconsistencies_introduced,
        "new_orphans": new_orphans,
        "git_operation_in_progress": git_op_in_progress,
    });

    Response {
        ok: true,
        body: body.to_string(),
        error: None,
    }
}

/// Return all edges that cross a language boundary (from-node and to-node
/// are in different source languages, as determined by file extension).
fn cached_cross_lang(_params: &Value, g: &SymbolGraph) -> Response {
    use coregraph_core::Language;

    // First pass: collect raw records so we can sort before JSON construction.
    // Sorting on the final `Value` would require a custom `Ord` over serde
    // values; materialising tuples keeps the key explicit and cheap.
    struct Record {
        from_name: String,
        from_lang: String,
        to_name: String,
        to_lang: String,
        kind: String,
        confidence: f64,
    }

    let mut records: Vec<Record> = g
        .edges()
        .filter_map(|e| {
            let from = g.get_node(e.from)?;
            let to = g.get_node(e.to)?;
            let from_lang = Language::from_path(&from.file);
            let to_lang = Language::from_path(&to.file);
            if from_lang != to_lang {
                Some(Record {
                    from_name: from.name.clone(),
                    from_lang: format!("{:?}", from_lang),
                    to_name: to.name.clone(),
                    to_lang: format!("{:?}", to_lang),
                    kind: format!("{:?}", e.kind),
                    confidence: e.current_confidence(),
                })
            } else {
                None
            }
        })
        .collect();
    // Sort by (from_name, to_name, kind) so two calls on the same graph
    // produce byte-identical bodies. petgraph's edge iteration order is
    // probably stable today but relying on it would be fragile.
    records.sort_by(|a, b| {
        (&a.from_name, &a.to_name, &a.kind).cmp(&(&b.from_name, &b.to_name, &b.kind))
    });

    let edges: Vec<_> = records
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "from": r.from_name,
                "from_lang": r.from_lang,
                "to": r.to_name,
                "to_lang": r.to_lang,
                "kind": r.kind,
                "confidence": r.confidence,
            })
        })
        .collect();

    Response {
        ok: true,
        body: serde_json::json!({"edges": edges}).to_string(),
        error: None,
    }
}

/// Compute impact for each named symbol in the `symbols` list and return
/// per-symbol results keyed by name.
///
/// Confidence-weighted impact is summed manually from `ImpactResult::edges`
/// (there is no pre-computed field on `ImpactResult` for this). When multiple
/// nodes share a name (e.g. overloaded functions in different files) all seeds
/// are accumulated into a single result entry.
fn cached_impact_batch(params: &Value, g: &SymbolGraph) -> Response {
    let symbols: Vec<String> = params
        .get("symbols")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Guard against runaway requests: reject batches larger than the cap
    // rather than silently truncating. A client sending more is almost
    // certainly mis-configured.
    if symbols.len() > MAX_SYMBOLS_PER_BATCH {
        return Response {
            ok: false,
            body: String::new(),
            error: Some(format!(
                "impact_batch accepts at most {} symbols per request; got {}",
                MAX_SYMBOLS_PER_BATCH,
                symbols.len()
            )),
        };
    }

    let mut results = serde_json::Map::new();
    for sym in symbols {
        let seed_ids: Vec<_> = g.nodes().filter(|n| n.name == sym).map(|n| n.id).collect();
        if seed_ids.is_empty() {
            results.insert(sym, serde_json::Value::Null);
            continue;
        }
        // Per-name seed cap: names like `new`, `init`, `handle` can match
        // dozens of nodes across a large project. Running a depth-3 impact
        // walk per seed would balloon a single IPC call into 50+ synchronous
        // traversals. Cap at `MAX_SEEDS_PER_NAME` and expose the original
        // count plus a `truncated` flag so the client can detect the cap.
        let original_seed_count = seed_ids.len();
        let capped_len = original_seed_count.min(MAX_SEEDS_PER_NAME);
        let truncated = capped_len < original_seed_count;

        let mut total_conf = 0.0f64;
        let mut total_edges = 0usize;
        let mut total_nodes = 0usize;
        for id in seed_ids.iter().take(capped_len) {
            let res = compute_impact(g, *id, 3);
            total_conf += res
                .edges
                .iter()
                .map(|e| e.current_confidence())
                .sum::<f64>();
            total_edges += res.edge_count();
            total_nodes += res.node_count();
        }
        results.insert(
            sym,
            serde_json::json!({
                "seeds": original_seed_count,
                "seeds_used": capped_len,
                "truncated": truncated,
                "nodes": total_nodes,
                "edges": total_edges,
                "confidence_weighted": total_conf,
            }),
        );
    }
    Response {
        ok: true,
        body: serde_json::json!({"results": results}).to_string(),
        error: None,
    }
}

fn dispatch_query(params: &Value, project: &Path) -> Response {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() {
        return Response {
            ok: false,
            body: String::new(),
            error: Some("missing 'name'".into()),
        };
    }
    match build_graph(project) {
        Ok((g, _)) => {
            let result = query_symbol(name, &g);
            let body = serde_json::json!({
                "name": name,
                "count": result.matches.len(),
                "matches": result.matches.iter().map(|n| serde_json::json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": format!("{:?}", n.kind),
                    "file": n.file.display().to_string(),
                    "span_start": n.span_start,
                    "span_end": n.span_end,
                })).collect::<Vec<_>>(),
            });
            Response {
                ok: true,
                body: body.to_string(),
                error: None,
            }
        }
        Err(e) => Response {
            ok: false,
            body: String::new(),
            error: Some(e.to_string()),
        },
    }
}

fn dispatch_impact(params: &Value, project: &Path) -> Response {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    if name.is_empty() {
        return Response {
            ok: false,
            body: String::new(),
            error: Some("missing 'name'".into()),
        };
    }
    match build_graph(project) {
        Ok((g, _)) => {
            let Some(seed) = g.nodes().find(|n| n.name == name).cloned() else {
                return Response {
                    ok: true,
                    body: serde_json::json!({"name": name, "reachable": 0}).to_string(),
                    error: None,
                };
            };
            let result = compute_impact(&g, seed.id, depth);
            let body = serde_json::json!({
                "name": name,
                "reachable": result.reachable.len(),
                "edges": result.edges.len(),
                "depth": result.depth_reached,
            });
            Response {
                ok: true,
                body: body.to_string(),
                error: None,
            }
        }
        Err(e) => Response {
            ok: false,
            body: String::new(),
            error: Some(e.to_string()),
        },
    }
}

fn dispatch_orphans(params: &Value, project: &Path) -> Response {
    match build_graph(project) {
        // Delegate to the shared cached handler with the project root so the
        // non-daemon path applies the same public_only/exclude_tests filtering
        // and library-API labelling as the daemon path.
        Ok((g, _)) => cached_orphans(params, &g, Some(project)),
        Err(e) => Response {
            ok: false,
            body: String::new(),
            error: Some(e.to_string()),
        },
    }
}

fn dispatch_inconsistencies(_params: &Value, project: &Path) -> Response {
    match build_graph(project) {
        Ok((g, _)) => {
            let reports = find_inconsistencies(&g);
            let body = serde_json::json!({
                "count": reports.len(),
                "reports": reports.iter().map(|r| serde_json::json!({
                    "shared_value": r.shared_value,
                    "a": {"name": r.node_a.name, "file": r.node_a.file.display().to_string()},
                    "b": {"name": r.node_b.name, "file": r.node_b.file.display().to_string()},
                })).collect::<Vec<_>>(),
            });
            Response {
                ok: true,
                body: body.to_string(),
                error: None,
            }
        }
        Err(e) => Response {
            ok: false,
            body: String::new(),
            error: Some(e.to_string()),
        },
    }
}

fn dispatch_stats(_params: &Value, project: &Path) -> Response {
    match build_graph(project) {
        Ok((g, files)) => {
            let body = serde_json::json!({
                "files": files,
                "symbols": g.node_count(),
                "edges": g.edge_count(),
            });
            Response {
                ok: true,
                body: body.to_string(),
                error: None,
            }
        }
        Err(e) => Response {
            ok: false,
            body: String::new(),
            error: Some(e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    /// Build a graph with one TypeScript node ("Caller" in caller.ts) and one
    /// Rust node ("Callee" in callee.rs) connected by a Calls edge. Exercises
    /// the cross_lang filter — the two nodes disagree on language-from-path.
    fn fixture_graph_with_cross_lang_edge() -> SymbolGraph {
        let mut g = SymbolGraph::new();
        let ts_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Caller",
            PathBuf::from("/proj/caller.ts"),
            10,
            16,
        ));
        let rs_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Callee",
            PathBuf::from("/proj/callee.rs"),
            10,
            16,
        ));
        g.insert_edge(DirectEdge::new(
            ts_id,
            rs_id,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.85),
            PathBuf::from("/proj/caller.ts"),
        ));
        g
    }

    fn fixture_graph() -> SymbolGraph {
        // Two symbols of the same name in different files plus a `Calls`
        // edge between them — exercises the "name match + edge fan-out"
        // code paths used by `definition` and `references`.
        let mut g = SymbolGraph::new();
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Foo",
            PathBuf::from("/proj/a.rs"),
            10,
            13,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Foo",
            PathBuf::from("/proj/b.rs"),
            42,
            45,
        ));
        let c = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Bar",
            PathBuf::from("/proj/b.rs"),
            100,
            103,
        ));
        g.insert_edge(DirectEdge::new(
            c,
            a,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.85),
            PathBuf::from("/proj/b.rs"),
        ));
        let _ = b;
        g
    }

    #[test]
    fn cached_definition_returns_every_match() {
        let g = fixture_graph();
        let resp = cached_definition(&serde_json::json!({"symbol": "Foo"}), &g);
        assert!(resp.ok);
        let body: Value = serde_json::from_str(&resp.body).unwrap();
        let locations = body["locations"].as_array().unwrap();
        assert_eq!(locations.len(), 2, "two `Foo` definitions");
        let files: Vec<&str> = locations
            .iter()
            .map(|l| l["file"].as_str().unwrap())
            .collect();
        assert!(files.contains(&"/proj/a.rs"));
        assert!(files.contains(&"/proj/b.rs"));
    }

    #[test]
    fn cached_definition_rejects_empty_symbol() {
        let g = fixture_graph();
        let resp = cached_definition(&serde_json::json!({"symbol": ""}), &g);
        assert!(!resp.ok, "empty symbol must error, not silently succeed");
    }

    #[test]
    fn cached_references_returns_neighbors_via_edges() {
        let g = fixture_graph();
        let resp = cached_references(&serde_json::json!({"symbol": "Foo"}), &g);
        assert!(resp.ok);
        let body: Value = serde_json::from_str(&resp.body).unwrap();
        let locations = body["locations"].as_array().unwrap();
        // `Bar` calls `Foo`, so a reference query for Foo should
        // surface `Bar` as the connected node.
        let names: Vec<&str> = locations
            .iter()
            .filter_map(|l| l.get("file").and_then(|f| f.as_str()))
            .collect();
        assert!(names.iter().any(|f| f.contains("b.rs")));
    }

    #[test]
    fn cached_workspace_symbol_substring_match() {
        let g = fixture_graph();
        let resp = cached_workspace_symbol(&serde_json::json!({"query": "Foo"}), &g);
        assert!(resp.ok);
        let body: Value = serde_json::from_str(&resp.body).unwrap();
        let symbols = body["symbols"].as_array().unwrap();
        assert_eq!(symbols.len(), 2);
        for s in symbols {
            assert_eq!(s["name"].as_str().unwrap(), "Foo");
        }
    }

    #[test]
    fn cached_workspace_symbol_empty_query_returns_all_capped() {
        let g = fixture_graph();
        let resp = cached_workspace_symbol(&serde_json::json!({"query": ""}), &g);
        assert!(resp.ok);
        let body: Value = serde_json::from_str(&resp.body).unwrap();
        let symbols = body["symbols"].as_array().unwrap();
        assert_eq!(symbols.len(), 3);
    }

    #[test]
    fn dispatch_cached_routes_lsp_methods() {
        // Confirms the dispatcher accepts the new method names rather
        // than returning an "unknown method" error. The handler-level
        // tests above already verify response shape.
        let g = fixture_graph();
        for method in ["lsp.definition", "lsp.references", "lsp.workspace_symbol"] {
            let params = if method == "lsp.workspace_symbol" {
                serde_json::json!({"query": "Foo"})
            } else {
                serde_json::json!({"symbol": "Foo"})
            };
            let resp = dispatch_cached(method, &params, &g);
            assert!(resp.ok, "{} dispatch failed: {:?}", method, resp.error);
        }
    }

    /// Build a graph whose single node points into a real file on disk.
    /// Places 'target' at byte offset 10 → (line 1, col 3).
    /// Content: "// pad\nfn target() {}\n"
    ///           0123456 7 890123456789
    ///
    /// The temp path includes a nanosecond-scale nonce in addition to
    /// the PID so parallel `cargo test` runs inside the same process
    /// cannot collide on file name or on the process-wide
    /// `range_cache()` entry (which is keyed by absolute path).
    fn fixture_graph_with_real_file() -> (SymbolGraph, std::path::PathBuf) {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let temp = std::env::temp_dir().join(format!(
            "cg-precise-range-test-{}-{}.rs",
            std::process::id(),
            nonce,
        ));
        std::fs::write(&temp, "// pad\nfn target() {}\n").unwrap();

        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(1),
            SymbolKind::Function,
            "target",
            temp.clone(),
            10, // "// pad\n" = 7 bytes, then "fn " = 3 bytes → offset 10 points to 't' of target
            16, // "target" is 6 chars → span_end = 16
        ));
        (g, temp)
    }

    /// Removes the temp fixture on drop. Also invalidates the
    /// process-wide `range_cache` entry for the path so a later test
    /// that reuses the same path (e.g. after a nonce collision at the
    /// nanosecond boundary) cannot read stale cached content.
    struct TempFileGuard(std::path::PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            range_cache().invalidate(&self.0);
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn cached_definition_returns_precise_range_for_known_symbol() {
        let (g, temp) = fixture_graph_with_real_file();
        let _guard = TempFileGuard(temp.clone());

        let resp = dispatch_cached(
            "lsp.definition",
            &serde_json::json!({"symbol": "target"}),
            &g,
        );
        assert!(resp.ok, "dispatch should succeed: {:?}", resp.error);

        let body: Value = serde_json::from_str(&resp.body).unwrap();
        let locations = body["locations"].as_array().unwrap();
        assert!(!locations.is_empty(), "expected at least one location");

        // offset 10 in "// pad\nfn target()..." is line 1, col 3
        let range = &locations[0]["range"];
        assert_eq!(
            range["start"]["line"].as_u64(),
            Some(1),
            "expected start.line == 1"
        );
        assert_eq!(
            range["start"]["character"].as_u64(),
            Some(3),
            "expected start.character == 3"
        );
    }

    #[test]
    fn dispatch_cached_inspect_no_match_returns_null_symbol() {
        use serde_json::json;
        let g = fixture_graph();
        let resp = dispatch_cached(
            "inspect",
            &json!({"file": "foo.rs", "line": 1, "column": 0}),
            &g,
        );
        assert!(resp.ok, "inspect should succeed: {:?}", resp.error);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        // The fixture paths (/proj/a.rs, /proj/b.rs) don't exist on disk,
        // and the query uses "foo.rs" which doesn't match them, so the
        // handler returns the no-match branch with symbol: null.
        // Value::Null is still Some(value) when accessed via .get() on the
        // JSON object — .get("symbol").is_some() checks key existence, not
        // whether the value is non-null.
        assert!(body.get("symbol").is_some());
        assert_eq!(body["symbol"], serde_json::Value::Null);
        assert!(body.get("edges_in").is_some());
        assert!(body.get("edges_out").is_some());
        assert!(body.get("freshness").is_some());
    }

    #[test]
    fn dispatch_cached_inspect_happy_path_returns_symbol_details() {
        use serde_json::json;
        let (g, temp) = fixture_graph_with_real_file();
        let _guard = TempFileGuard(temp.clone());

        // The fixture writes "// pad\nfn target() {}\n" and places the
        // `target` node at byte offset 10, which resolves to (line 1, col 3).
        // Pass the full temp path so Path::ends_with matches the node's file.
        let resp = dispatch_cached(
            "inspect",
            &json!({
                "file": temp.display().to_string(),
                "line": 1,
                "column": 0,
            }),
            &g,
        );
        assert!(resp.ok, "inspect should succeed: {:?}", resp.error);

        let body: Value = serde_json::from_str(&resp.body).unwrap();
        let symbol = body
            .get("symbol")
            .expect("symbol key must exist in happy-path response");
        assert!(
            !symbol.is_null(),
            "symbol should be an object, got null: {}",
            resp.body
        );
        assert_eq!(symbol["name"].as_str(), Some("target"));
        assert!(
            symbol["file"].is_string(),
            "file should be a string, got {:?}",
            symbol["file"]
        );
        assert_eq!(symbol["line"].as_u64(), Some(1));

        // edges_in/edges_out must be arrays (empty is fine for single-node).
        assert!(
            body["edges_in"].is_array(),
            "edges_in should be an array, got {:?}",
            body["edges_in"]
        );
        assert!(
            body["edges_out"].is_array(),
            "edges_out should be an array, got {:?}",
            body["edges_out"]
        );

        // freshness.last_rebuild_at_ms must be a number.
        assert!(
            body["freshness"]["last_rebuild_at_ms"].is_number(),
            "last_rebuild_at_ms should be a number, got {:?}",
            body["freshness"]["last_rebuild_at_ms"]
        );
    }

    #[test]
    fn dispatch_cached_reindex_unknown_mode_returns_error() {
        use serde_json::json;
        let g = fixture_graph();
        let resp = dispatch_cached("reindex", &json!({"mode": "banana"}), &g);
        assert!(!resp.ok);
        assert!(resp.error.as_ref().unwrap().contains("mode"));
    }

    #[test]
    fn dispatch_cached_reindex_full_returns_ok_false_with_clear_error() {
        use serde_json::json;
        let g = fixture_graph();
        let resp = dispatch_cached("reindex", &json!({"mode": "full"}), &g);
        assert!(!resp.ok);
        assert!(resp
            .error
            .as_ref()
            .unwrap()
            .contains("reindex requires mutable graph state"));
    }

    #[test]
    fn dispatch_cached_diff_returns_list_shape() {
        use serde_json::json;
        let g = fixture_graph();
        let resp = dispatch_cached("diff", &json!({"base_ref": "HEAD"}), &g);
        assert!(resp.ok);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        assert!(body
            .get("impacted_symbols")
            .and_then(|v| v.as_array())
            .is_some());
        assert!(body
            .get("inconsistencies_introduced")
            .and_then(|v| v.as_array())
            .is_some());
    }

    #[test]
    fn dispatch_cached_cross_lang_returns_edges_list() {
        use serde_json::json;
        let g = fixture_graph_with_cross_lang_edge();
        let resp = dispatch_cached("cross_lang", &json!({}), &g);
        assert!(resp.ok);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        let edges = body["edges"].as_array().unwrap();
        assert!(
            !edges.is_empty(),
            "expected at least one cross-language edge"
        );
    }

    #[test]
    fn dispatch_cached_impact_batch_returns_per_symbol_results() {
        use serde_json::json;
        let g = fixture_graph();
        let resp = dispatch_cached("impact_batch", &json!({"symbols": ["Foo", "Bar"]}), &g);
        assert!(resp.ok);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        let results = body["results"].as_object().unwrap();
        assert!(results.contains_key("Foo"));
        assert!(results.contains_key("Bar"));
    }

    #[test]
    fn dispatch_cached_impact_batch_empty_symbols_returns_empty_results() {
        use serde_json::json;
        let g = fixture_graph();
        let resp = dispatch_cached("impact_batch", &json!({"symbols": []}), &g);
        assert!(resp.ok, "empty batch should succeed: {:?}", resp.error);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        let results = body["results"].as_object().unwrap();
        assert!(results.is_empty(), "results should be empty");
    }

    #[test]
    fn dispatch_cached_impact_batch_unknown_symbol_returns_null() {
        use serde_json::json;
        let g = fixture_graph();
        let resp = dispatch_cached("impact_batch", &json!({"symbols": ["Nonexistent"]}), &g);
        assert!(resp.ok);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        let results = body["results"].as_object().unwrap();
        // Key must exist, value must be JSON null (signals "name not found").
        assert!(results.contains_key("Nonexistent"));
        assert_eq!(results["Nonexistent"], serde_json::Value::Null);
    }

    #[test]
    fn dispatch_cached_impact_batch_over_limit_returns_error() {
        use serde_json::json;
        let g = fixture_graph();
        // 65 symbols — one past the cap.
        let symbols: Vec<&str> = std::iter::repeat_n("x", 65).collect();
        let resp = dispatch_cached("impact_batch", &json!({"symbols": symbols}), &g);
        assert!(!resp.ok, "over-limit batch must reject");
        let err = resp.error.as_ref().expect("must carry error message");
        assert!(
            err.contains("at most 64"),
            "error should mention the cap; got: {}",
            err
        );
    }

    #[test]
    fn dispatch_cached_diff_ordering_stable() {
        use serde_json::json;
        let g = fixture_graph();
        let a = dispatch_cached("diff", &json!({"base_ref": "HEAD"}), &g);
        let b = dispatch_cached("diff", &json!({"base_ref": "HEAD"}), &g);
        assert!(a.ok && b.ok);
        // Byte-equal bodies across repeated calls prove the sort keeps the
        // shape deterministic even when the underlying HashSet or iteration
        // order would otherwise vary.
        assert_eq!(a.body, b.body, "diff body must be deterministic");
    }

    /// Verify that `dispatch_diff_with_git` returns `ok:true` and the
    /// correct response shape even when the supplied directory is not a git
    /// repository. The graceful-degradation branch should emit empty arrays
    /// rather than panicking or returning `ok:false`.
    #[test]
    fn dispatch_diff_with_git_reports_changed_files_shape() {
        use serde_json::json;
        // Use a temp directory that is guaranteed not to be a git repo so
        // the git-failure code path exercises graceful degradation.
        let dir = std::env::temp_dir().join(format!("cg-diff-test-{}-{}", std::process::id(), {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        }));
        std::fs::create_dir_all(&dir).ok();
        let g = fixture_graph();
        let resp = super::dispatch_diff_with_git(&json!({"base_ref": "HEAD"}), &g, &dir);
        assert!(
            resp.ok,
            "diff must succeed even on non-repo dir: {:?}",
            resp.error
        );
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        // Required top-level fields must always be present.
        assert!(
            body["changed_files"].is_array(),
            "changed_files must be an array"
        );
        assert_eq!(
            body["base_ref"].as_str(),
            Some("HEAD"),
            "base_ref must echo the param"
        );
        assert!(
            body["total_reachable"].is_number(),
            "total_reachable must be a number"
        );
        assert!(
            body["inconsistencies_introduced"].is_array(),
            "inconsistencies_introduced must be an array"
        );
        assert!(
            body["new_orphans"].is_array(),
            "new_orphans must be an array"
        );
        assert!(
            body.get("git_operation_in_progress").is_some(),
            "git_operation_in_progress must be present"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dispatch_cached_cross_lang_no_cross_edges_returns_empty() {
        use serde_json::json;
        // fixture_graph() has only Rust-paths (.rs); no cross-language edges.
        let g = fixture_graph();
        let resp = dispatch_cached("cross_lang", &json!({}), &g);
        assert!(resp.ok);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        let edges = body["edges"].as_array().expect("edges must be an array");
        assert!(
            edges.is_empty(),
            "single-language graph should yield no cross-lang edges, got {:?}",
            edges
        );
    }

    // ---------------------------------------------------------------------------
    // dispatch() reindex tests (uncached path)
    // ---------------------------------------------------------------------------

    /// Workspace root used by reindex tests. Resolves two levels up from the
    /// CLI crate manifest dir to the repo root so build_graph has a real
    /// project to scan.
    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf()
    }

    #[test]
    #[ignore] // slow: rebuilds the workspace graph; run with `cargo test -- --ignored`
    fn dispatch_reindex_full_succeeds_on_real_project() {
        use serde_json::json;
        let project = workspace_root();
        let resp = dispatch("reindex", &json!({"mode": "full"}), &project);
        assert!(resp.ok, "reindex full should succeed: {:?}", resp.error);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(body["mode"], "full");
        assert!(
            body.get("elapsed_ms").is_some(),
            "response must include elapsed_ms"
        );
        assert!(
            body.get("node_count").and_then(|v| v.as_u64()).is_some(),
            "response must include a numeric node_count"
        );
    }

    #[test]
    fn dispatch_reindex_fast_without_daemon_returns_error() {
        use serde_json::json;
        let project = workspace_root();
        let resp = dispatch(
            "reindex",
            &json!({"mode": "fast", "file": "crates/cli/src/dispatch.rs"}),
            &project,
        );
        assert!(
            !resp.ok,
            "fast reindex in the uncached path must fail honestly"
        );
        let err = resp.error.as_ref().unwrap();
        assert!(
            err.contains("daemon") || err.contains("fast mode"),
            "error message should point caller toward the running-daemon path: got {err}"
        );
    }

    #[test]
    fn dispatch_reindex_invalid_mode_returns_error() {
        use serde_json::json;
        let project = workspace_root();
        let resp = dispatch("reindex", &json!({"mode": "banana"}), &project);
        assert!(!resp.ok);
        let err = resp.error.as_ref().unwrap();
        assert!(err.contains("mode"), "error should mention 'mode': {err}");
    }

    #[test]
    fn dispatch_reindex_fast_missing_file_param_returns_error() {
        use serde_json::json;
        let project = workspace_root();
        let resp = dispatch("reindex", &json!({"mode": "fast"}), &project);
        assert!(!resp.ok);
        // The fast branch requires a file; the error should name the parameter.
        assert!(
            resp.error.as_ref().unwrap().to_lowercase().contains("file"),
            "error should mention 'file': {:?}",
            resp.error
        );
    }

    // ---------------------------------------------------------------------------
    // dispatch_reindex_mutable tests (daemon write-path)
    // ---------------------------------------------------------------------------

    #[test]
    fn dispatch_reindex_mutable_full_rebuilds_and_marks_fresh() {
        use std::time::Duration;
        let (g, project) = fixture_graph_with_real_file();
        let _guard = TempFileGuard(project.clone());

        // Age the graph artificially so mark_full_rebuild has something to reset.
        let mut g = g;
        std::thread::sleep(Duration::from_millis(2));

        // build_graph on a single-file temp isn't a real crate; this may fail.
        // If so, the test asserts the error path instead. The goal here is a
        // compile-time smoke check and basic API contract verification.
        let params = serde_json::json!({"mode": "full"});
        let project_dir = project.parent().unwrap().to_path_buf();
        let resp = super::dispatch_reindex_mutable(&params, &mut g, &project_dir);

        // Either Ok (graph replaced) or a clear error — both are acceptable.
        if resp.ok {
            let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
            assert_eq!(body["mode"], "full");
            assert!(
                body.get("node_count").is_some(),
                "ok response must include node_count"
            );
        } else {
            assert!(
                resp.error.is_some(),
                "failed response must carry an error message"
            );
        }
    }

    #[test]
    fn dispatch_reindex_mutable_fast_requires_file_param() {
        let mut g = fixture_graph();
        let project = workspace_root();
        let params = serde_json::json!({"mode": "fast"});
        let resp = super::dispatch_reindex_mutable(&params, &mut g, &project);
        assert!(!resp.ok);
        let err = resp.error.as_ref().unwrap();
        assert!(
            err.to_lowercase().contains("file"),
            "error should mention 'file': {err}"
        );
    }

    #[test]
    fn dispatch_reindex_mutable_invalid_mode_returns_error() {
        let mut g = fixture_graph();
        let project = workspace_root();
        let params = serde_json::json!({"mode": "turbo"});
        let resp = super::dispatch_reindex_mutable(&params, &mut g, &project);
        assert!(!resp.ok);
        let err = resp.error.as_ref().unwrap();
        assert!(err.contains("mode"), "error should mention 'mode': {err}");
    }

    #[test]
    fn dispatch_reindex_mutable_fast_missing_file_returns_file_missing_true() {
        use serde_json::json;

        // Point at a temp path that doesn't exist — fast-path should
        // succeed with file_missing: true (deletion scenario).
        let mut g = fixture_graph();
        let project = workspace_root();
        let gone_path = std::env::temp_dir().join("cg-no-such-file-12345.rs");
        let params = json!({"mode": "fast", "file": gone_path.display().to_string()});
        let resp = super::dispatch_reindex_mutable(&params, &mut g, &project);
        assert!(
            resp.ok,
            "missing file is a valid deletion scenario: {:?}",
            resp.error
        );
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(
            body["file_missing"], true,
            "file_missing should be true for a gone file"
        );
        assert_eq!(body["mode"], "fast");
    }

    // ---------------------------------------------------------------------------
    // reindex_file_in_place cross-file re-link tests (0.5c)
    // ---------------------------------------------------------------------------

    /// Build a 2-file graph in memory, write the callee file to disk so the
    /// extractor can re-read it, then call `reindex_file_in_place` and verify
    /// that the cross-file `caller → callee` edge is re-linked with
    /// `stale_evidence_count == 1`.
    #[test]
    fn reindex_file_in_place_relinks_cross_file_edges_with_stale_bump() {
        use std::time::{SystemTime, UNIX_EPOCH};

        // Unique temp dir per test run to avoid collisions under parallel test.
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("cg-relink-test-{}-{}", std::process::id(), nonce));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let callee_file = temp_dir.join("callee.rs");

        // Write a minimal Rust file so the Rust extractor can extract "callee".
        std::fs::write(&callee_file, "pub fn callee() {}\n").unwrap();

        let caller_file = PathBuf::from("caller.rs"); // synthetic — no disk file needed

        let mut g = SymbolGraph::new();

        // caller node in caller.rs (synthetic, stays in the graph unchanged).
        let caller_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "caller",
            caller_file.clone(),
            0,
            10,
        ));

        // callee node in the temp file — simulates the pre-reindex state.
        let callee_old_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "callee",
            callee_file.clone(),
            7,
            13,
        ));

        // Cross-file edge: caller → callee. Evidence lives in caller.rs (not
        // the reindexed file) so it qualifies as a cross-file edge.
        g.insert_edge(coregraph_core::DirectEdge::new(
            caller_id,
            callee_old_id,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.85),
            caller_file.clone(),
        ));

        // Sanity: the edge should be fresh (stale_evidence_count == 0).
        let pre_edge = g
            .edges()
            .find(|e| e.from == caller_id)
            .expect("caller → callee edge should exist pre-reindex");
        assert_eq!(
            pre_edge.stale_evidence_count, 0,
            "fresh edge should have stale_evidence_count == 0"
        );

        // Run the surgical reindex of the callee file.
        let started = std::time::Instant::now();
        let resp = super::reindex_file_in_place(&mut g, &callee_file, started);

        // Cleanup the temp dir regardless of assertions below.
        let _ = std::fs::remove_dir_all(&temp_dir);

        assert!(
            resp.ok,
            "reindex_file_in_place should succeed: {:?}",
            resp.error
        );

        let body: serde_json::Value =
            serde_json::from_str(&resp.body).expect("response body should be valid JSON");
        assert_eq!(body["mode"], "fast", "mode field should be 'fast'");

        // The cross-file caller → callee edge must be re-linked.
        let relinked = g.edges().find(|e| e.from == caller_id);
        assert!(
            relinked.is_some(),
            "caller → callee edge should be re-linked after reindex"
        );
        let relinked = relinked.unwrap();
        assert_eq!(
            relinked.stale_evidence_count, 1,
            "re-linked edge stale_evidence_count must be bumped to 1"
        );

        // Response telemetry must reflect the re-link.
        assert_eq!(
            body["cross_file_edges_staled"].as_u64(),
            Some(1),
            "response must report cross_file_edges_staled == 1"
        );
        assert_eq!(
            body["cross_file_edges_dropped"].as_u64(),
            Some(0),
            "response must report cross_file_edges_dropped == 0"
        );

        // The new callee node in the graph must be different from the old id
        // (fresh SymbolId allocated by insert_node).
        let new_callee_id = relinked.to;
        assert_ne!(
            new_callee_id, callee_old_id,
            "new callee node id should differ from the old one"
        );

        // The new callee node must live in the callee file.
        let new_callee = g
            .get_node(new_callee_id)
            .expect("new callee node must be in the graph");
        assert_eq!(
            new_callee.file.as_ref(),
            callee_file.as_path(),
            "re-linked edge must point to a node in the reindexed file"
        );
        assert_eq!(
            new_callee.name, "callee",
            "new callee node must be named 'callee'"
        );
    }

    /// Verify that when the callee symbol is absent from the rewritten file,
    /// the edge is dropped and reported in cross_file_edges_dropped.
    #[test]
    fn reindex_file_in_place_drops_edge_when_symbol_removed() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("cg-drop-test-{}-{}", std::process::id(), nonce));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let callee_file = temp_dir.join("callee.rs");

        // Write a file that does NOT define "callee" — simulates a rename.
        std::fs::write(&callee_file, "pub fn renamed_callee() {}\n").unwrap();

        let caller_file = PathBuf::from("caller.rs");
        let mut g = SymbolGraph::new();

        let caller_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "caller",
            caller_file.clone(),
            0,
            10,
        ));
        let callee_old_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "callee",
            callee_file.clone(),
            7,
            13,
        ));
        g.insert_edge(coregraph_core::DirectEdge::new(
            caller_id,
            callee_old_id,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.85),
            caller_file.clone(),
        ));

        let started = std::time::Instant::now();
        let resp = super::reindex_file_in_place(&mut g, &callee_file, started);
        let _ = std::fs::remove_dir_all(&temp_dir);

        assert!(resp.ok);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();

        // The old callee symbol is gone; edge cannot be re-linked.
        assert_eq!(
            body["cross_file_edges_dropped"].as_u64(),
            Some(1),
            "dropped edge must be reported when symbol is renamed/removed"
        );
        assert_eq!(
            body["cross_file_edges_staled"].as_u64(),
            Some(0),
            "no re-links should succeed when symbol is gone"
        );

        // The caller → old_callee edge must not exist.
        let dangling = g.edges().find(|e| e.from == caller_id);
        assert!(
            dangling.is_none(),
            "dangling edge must not exist in the graph after dropped re-link"
        );
    }

    /// Exercises the qualified-name lookup path: the captured node has a
    /// distinct `qualified_name` (e.g. `"mymod::target"`) while the bare
    /// `name` is `"target"`. After re-extraction the Rust extractor emits a
    /// new node with `name == qualified_name == "target"`. The re-link must
    /// succeed via the `lookup_by_name` fallback because the qname index
    /// holds the OLD `"mymod::target"` key which no extractor re-emits.
    ///
    /// This is the key regression test for NB-1: before the fix, the fallback
    /// called `lookup_by_name("mymod::target", 4)` which never matches
    /// anything (the name index is keyed by bare names). After the fix, the
    /// fallback uses the independently-captured `file_name` field.
    #[test]
    fn reindex_file_in_place_relinks_via_name_fallback_when_qname_key_changes() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("cg-relink-qname-{}-{}", std::process::id(), nonce));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let target_file = temp_dir.join("target.rs");
        std::fs::write(&target_file, "pub fn target() {}\n").unwrap();

        let caller_file = PathBuf::from("caller.rs");
        let mut g = SymbolGraph::new();

        let caller_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "caller",
            caller_file.clone(),
            0,
            10,
        ));

        // Pre-reindex target: bare name "target" but qualified_name
        // "mymod::target" — mimics what a Java/Python/Kotlin extractor would
        // produce. After reindex, the Rust extractor will emit a node with
        // name == qualified_name == "target" (no module qualifier), so the
        // OLD qname "mymod::target" will NOT match any new node.
        let target_old_id = g.insert_node(
            SymbolNode::new(
                SymbolId(0),
                SymbolKind::Function,
                "target",
                target_file.clone(),
                7,
                13,
            )
            .with_qualified_name("mymod::target"),
        );

        g.insert_edge(coregraph_core::DirectEdge::new(
            caller_id,
            target_old_id,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.85),
            caller_file.clone(),
        ));

        let started = std::time::Instant::now();
        let resp = super::reindex_file_in_place(&mut g, &target_file, started);
        let _ = std::fs::remove_dir_all(&temp_dir);

        assert!(resp.ok, "reindex should succeed: {:?}", resp.error);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();

        // The qname lookup for "mymod::target" will miss (no new node has
        // that qualified_name). The name lookup for "target" MUST hit the
        // newly-inserted node.
        assert_eq!(
            body["cross_file_edges_staled"].as_u64(),
            Some(1),
            "re-link must succeed via name fallback: {body}"
        );
        assert_eq!(
            body["cross_file_edges_dropped"].as_u64(),
            Some(0),
            "no edges should be dropped when bare-name fallback works"
        );

        let relinked = g
            .edges()
            .find(|e| e.from == caller_id)
            .expect("caller → target edge should be re-linked");
        assert_eq!(
            relinked.stale_evidence_count, 1,
            "re-linked edge stale_evidence_count must be 1"
        );

        // The new target node must have been matched by bare name.
        let new_target = g.get_node(relinked.to).expect("new target node must exist");
        assert_eq!(new_target.name, "target");
        assert_eq!(
            new_target.file.as_ref(),
            target_file.as_path(),
            "re-linked edge must point into the reindexed file"
        );
    }

    /// Exercises the qualified-name primary lookup path: the captured node
    /// has `qualified_name == "target"` (same as name — the Rust extractor's
    /// default). This path should work both before and after the NB-1 fix,
    /// since the qname lookup matches directly. Kept as a regression guard.
    #[test]
    fn reindex_file_in_place_relinks_via_qname_primary_lookup() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "cg-relink-qname-primary-{}-{}",
            std::process::id(),
            nonce
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let target_file = temp_dir.join("target.rs");
        std::fs::write(&target_file, "pub fn target() {}\n").unwrap();

        let caller_file = PathBuf::from("caller.rs");
        let mut g = SymbolGraph::new();

        let caller_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "caller",
            caller_file.clone(),
            0,
            10,
        ));
        // qualified_name == name == "target" — matches what Rust extractor emits.
        let target_old_id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "target",
            target_file.clone(),
            7,
            13,
        ));

        g.insert_edge(coregraph_core::DirectEdge::new(
            caller_id,
            target_old_id,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.85),
            caller_file.clone(),
        ));

        let started = std::time::Instant::now();
        let resp = super::reindex_file_in_place(&mut g, &target_file, started);
        let _ = std::fs::remove_dir_all(&temp_dir);

        assert!(resp.ok);
        let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(
            body["cross_file_edges_staled"].as_u64(),
            Some(1),
            "qname primary lookup must succeed"
        );
        assert_eq!(body["cross_file_edges_dropped"].as_u64(), Some(0));
    }
}
