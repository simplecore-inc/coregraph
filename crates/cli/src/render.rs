//! Rich output renderers matching docs/cli.md §11 and §12.
//! - Human mode: Unicode box-drawing + ANSI colors + interactive footer
//! - LLM mode: markdown tables + actions/pagination hints
//! - JSON mode: structured schema with pagination/budget
//! - Actual depth-aware BFS + cursor pagination + token-budget truncation.

use coregraph_core::{DirectEdge, EdgeKind, SymbolNode};
use coregraph_graph::SymbolGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::global_opts::{ColorMode, OutputFormat};

/// 1-based line number of a byte offset within file `content` (counts newlines
/// before the offset). Clamps offsets past EOF to the last line.
fn line_of_byte(content: &[u8], byte: u32) -> usize {
    let end = (byte as usize).min(content.len());
    content[..end].iter().filter(|&&b| b == b'\n').count() + 1
}

/// 1-based source line of a node's `span_start` for the `file:line` locator in
/// query headers. Reads the file and counts newlines; returns 0 (unknown) when
/// the file can't be read. `inspect` consumes 1-based lines, so emitting the
/// raw byte offset here produced a confusing header AND a broken
/// `coregraph inspect file:<byte>` action suggestion.
fn header_line(file: &std::path::Path, span_start: u32) -> usize {
    std::fs::read(file)
        .map(|b| line_of_byte(&b, span_start))
        .unwrap_or(0)
}

/// Token-budget safety margin (docs/cli.md §12.1): actual = advertised × 0.7.
pub fn effective_budget(advertised: usize) -> usize {
    (advertised as f32 * 0.7) as usize
}

/// Byte-based token estimator (docs/cli.md §12.1).
pub fn estimate_tokens(text: &str) -> usize {
    let ascii = text.chars().filter(|c| c.is_ascii()).count();
    let non_ascii = text.chars().count() - ascii;
    (ascii / 4) + (non_ascii / 2) + 1
}

/// Budget zone fractions from docs/cli.md §12.2.
/// Used to cap each section of the output.
struct BudgetZones {
    header: usize,
    hop1: usize,
    hop_rest: usize,
    warnings: usize,
}
impl BudgetZones {
    fn for_total(total: usize) -> Self {
        let pct = |f: f32| (total as f32 * f) as usize;
        Self {
            header: pct(0.025),
            hop1: pct(0.30),
            hop_rest: pct(0.4375), // 2-hop(25) + 3-hop(18.75) combined
            warnings: pct(0.0625),
        }
    }
}

/// Opaque pagination cursor.
#[derive(Serialize, Deserialize, Debug)]
pub struct Cursor {
    pub page: usize,
    pub offset: usize,
}

pub fn encode_cursor(c: &Cursor) -> String {
    let json = serde_json::to_string(c).unwrap_or_default();
    base64_encode(json.as_bytes())
}
pub fn decode_cursor(token: &str) -> Option<Cursor> {
    let bytes = base64_decode(token)?;
    let s = std::str::from_utf8(&bytes).ok()?;
    serde_json::from_str(s).ok()
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(CHARS[((n >> 18) & 0x3f) as usize] as char);
        out.push(CHARS[((n >> 12) & 0x3f) as usize] as char);
        out.push(CHARS[((n >> 6) & 0x3f) as usize] as char);
        out.push(CHARS[(n & 0x3f) as usize] as char);
        i += 3;
    }
    match data.len() - i {
        1 => {
            let n = (data[i] as u32) << 16;
            out.push(CHARS[((n >> 18) & 0x3f) as usize] as char);
            out.push(CHARS[((n >> 12) & 0x3f) as usize] as char);
        }
        2 => {
            let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
            out.push(CHARS[((n >> 18) & 0x3f) as usize] as char);
            out.push(CHARS[((n >> 12) & 0x3f) as usize] as char);
            out.push(CHARS[((n >> 6) & 0x3f) as usize] as char);
        }
        _ => {}
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn idx(b: u8) -> Option<u32> {
        match b {
            b'A'..=b'Z' => Some((b - b'A') as u32),
            b'a'..=b'z' => Some((b - b'a' + 26) as u32),
            b'0'..=b'9' => Some((b - b'0' + 52) as u32),
            b'-' | b'+' => Some(62),
            b'_' | b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = input.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let a = idx(bytes[i])?;
        let b = idx(bytes[i + 1])?;
        let c = idx(bytes[i + 2])?;
        let d = idx(bytes[i + 3])?;
        let n = (a << 18) | (b << 12) | (c << 6) | d;
        out.push(((n >> 16) & 0xff) as u8);
        out.push(((n >> 8) & 0xff) as u8);
        out.push((n & 0xff) as u8);
        i += 4;
    }
    match bytes.len() - i {
        2 => {
            let a = idx(bytes[i])?;
            let b = idx(bytes[i + 1])?;
            let n = (a << 18) | (b << 12);
            out.push(((n >> 16) & 0xff) as u8);
        }
        3 => {
            let a = idx(bytes[i])?;
            let b = idx(bytes[i + 1])?;
            let c = idx(bytes[i + 2])?;
            let n = (a << 18) | (b << 12) | (c << 6);
            out.push(((n >> 16) & 0xff) as u8);
            out.push(((n >> 8) & 0xff) as u8);
        }
        _ => {}
    }
    Some(out)
}

/// An edge paired with the depth at which it was discovered via BFS from the center.
#[derive(Clone, Debug)]
pub struct EdgeAtDepth {
    pub edge: DirectEdge,
    pub depth: usize,
}

/// docs/cli.md §11.5 — path trust summary.
/// Computed over the collected edge set from a query.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathTrust {
    Verified,
    Broken {
        at_hop: usize,
        stale_file: String,
    },
    Partial {
        verified_paths: usize,
        broken_paths: usize,
    },
}

/// Heuristic: an edge is "verified" when its confidence >= 0.7 AND its
/// origin is NameResolved or CompilerDerived. Otherwise "broken".
fn edge_is_verified(e: &DirectEdge) -> bool {
    // Path-trust classification: every tier above pattern-guess counts as
    // verified. Previously only `NameResolved` / `CompilerDerived` passed,
    // which caused `SyntaxMatched` — the bread-and-butter identifier-based
    // resolution tier — to inflate the `broken_paths` counter. The UX
    // effect was a "7% verified, 93% broken" line on every query against
    // a stack-graphs-less language (Rust/Go/Kotlin), even though the
    // edges in question are regular identifier matches with high
    // precision.
    use coregraph_core::edge::AnalysisOrigin;
    e.confidence.0 >= 0.7
        && matches!(
            e.origin,
            AnalysisOrigin::NameResolved
                | AnalysisOrigin::CompilerDerived
                | AnalysisOrigin::SyntaxMatched
        )
}

/// Compute a PathTrust summary over the union of incoming + outgoing edges.
pub fn summarize_path_trust(incoming: &[EdgeAtDepth], outgoing: &[EdgeAtDepth]) -> PathTrust {
    let all: Vec<&EdgeAtDepth> = outgoing.iter().chain(incoming.iter()).collect();
    if all.is_empty() {
        return PathTrust::Verified;
    }
    let verified = all.iter().filter(|e| edge_is_verified(&e.edge)).count();
    let broken = all.len() - verified;
    if broken == 0 {
        PathTrust::Verified
    } else if verified == 0 {
        // Report the first unverified edge's location for diagnostic context.
        let first = all.iter().find(|e| !edge_is_verified(&e.edge));
        let (at_hop, stale_file) = match first {
            Some(e) => (e.depth, e.edge.evidence_file.display().to_string()),
            None => (0, String::new()),
        };
        PathTrust::Broken { at_hop, stale_file }
    } else {
        PathTrust::Partial {
            verified_paths: verified,
            broken_paths: broken,
        }
    }
}

/// Picks which definition to center a name query on when several symbols share
/// the queried name: prefer an exported/Public definition (the public API
/// surface) over module-private or test-local ones, which keeps the view on the
/// symbol the caller most likely means. Falls back to the first match.
pub fn pick_center_index(matches: &[&SymbolNode]) -> usize {
    // Prefer an exported/Public definition; among several Public same-name
    // defs, pick the lowest id so the chosen center is deterministic regardless
    // of name-index iteration order. With no Public match, fall back to the
    // first match (lookup order is node-insertion order, itself deterministic).
    matches
        .iter()
        .enumerate()
        .filter(|(_, n)| n.visibility == coregraph_core::Visibility::Public)
        .min_by_key(|(_, n)| n.id.0)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// When a name resolves to more than one *distinct* definition, builds a note
/// listing the definitions not chosen as center, so a precise (non-aggregated)
/// query never silently hides that other same-name symbols exist. Returns None
/// when the name is unique. Only definitions sharing the center's exact name
/// are listed — fuzzy substring matches with different names are excluded.
pub fn multi_def_note(matches: &[&SymbolNode], center_index: usize) -> Option<String> {
    let center = matches.get(center_index)?;
    let others: Vec<&SymbolNode> = matches
        .iter()
        .enumerate()
        .filter(|(i, n)| *i != center_index && n.name == center.name)
        .map(|(_, n)| *n)
        .collect();
    if others.is_empty() {
        return None;
    }
    let mut note = format!(
        "ℹ '{}' has {} definitions; showing the one at {}. \
         Re-run with --aggregate to include callers of the others:",
        center.name,
        others.len() + 1,
        center.file.display()
    );
    for n in others {
        note.push_str(&format!("\n    {} @ {}", n.name, n.file.display()));
    }
    Some(note)
}

/// Aggregates `bfs_edges_filtered` over a group of same-name symbol nodes
/// (overloads, or a module function and a method sharing the name) so that a
/// name query surfaces the callers of *every* matching definition, not just the
/// first one. Edges reachable from more than one center are de-duplicated by
/// `(from, to, kind)`; results are re-sorted by depth then edge-kind priority.
pub fn bfs_edges_aggregated(
    graph: &SymbolGraph,
    center_ids: &[coregraph_core::SymbolId],
    max_depth: usize,
    min_confidence: f32,
    include_stale: bool,
) -> (Vec<EdgeAtDepth>, Vec<EdgeAtDepth>) {
    let mut incoming: Vec<EdgeAtDepth> = Vec::new();
    let mut outgoing: Vec<EdgeAtDepth> = Vec::new();
    let mut seen_in: HashSet<(coregraph_core::SymbolId, coregraph_core::SymbolId, EdgeKind)> =
        HashSet::new();
    let mut seen_out: HashSet<(coregraph_core::SymbolId, coregraph_core::SymbolId, EdgeKind)> =
        HashSet::new();

    for &id in center_ids {
        let (inc, out) = bfs_edges_filtered(graph, id, max_depth, min_confidence, include_stale);
        for e in inc {
            if seen_in.insert((e.edge.from, e.edge.to, e.edge.kind.clone())) {
                incoming.push(e);
            }
        }
        for e in out {
            if seen_out.insert((e.edge.from, e.edge.to, e.edge.kind.clone())) {
                outgoing.push(e);
            }
        }
    }

    // Re-sort the merged sets the same way bfs_edges_filtered does: shallowest
    // hop first, then semantic edges (Calls/…) before structural/Resolves.
    let by_depth_then_kind = |a: &EdgeAtDepth, b: &EdgeAtDepth| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| edge_kind_priority(&a.edge.kind).cmp(&edge_kind_priority(&b.edge.kind)))
    };
    incoming.sort_by(by_depth_then_kind);
    outgoing.sort_by(by_depth_then_kind);
    (incoming, outgoing)
}

/// Same as bfs_edges but lets the caller exclude stale edges.
///
/// Edges are filtered by:
///   - confidence ≥ min_confidence
///   - staleness: stale edges are excluded unless `include_stale` is true
pub fn bfs_edges_filtered(
    graph: &SymbolGraph,
    center_id: coregraph_core::SymbolId,
    max_depth: usize,
    min_confidence: f32,
    include_stale: bool,
) -> (Vec<EdgeAtDepth>, Vec<EdgeAtDepth>) {
    let mut incoming: Vec<EdgeAtDepth> = Vec::new();
    let mut outgoing: Vec<EdgeAtDepth> = Vec::new();
    let mut visited_out: HashSet<coregraph_core::SymbolId> = HashSet::new();
    let mut visited_in: HashSet<coregraph_core::SymbolId> = HashSet::new();
    visited_out.insert(center_id);
    visited_in.insert(center_id);

    // An edge counts as "stale" when it was created before the latest
    // invalidation cut-off. A fresh build has no invalidations so nothing is
    // stale. Post-incremental-update, callers can exclude old evidence.
    let cutoff_epoch = graph.edges().map(|e| e.created_at_epoch).max().unwrap_or(0);
    let pass_stale =
        |e: &DirectEdge| -> bool { include_stale || !e.is_stale(cutoff_epoch, cutoff_epoch) };

    // Outgoing BFS
    let mut frontier = vec![center_id];
    for depth in 1..=max_depth.max(1) {
        let mut next: Vec<coregraph_core::SymbolId> = Vec::new();
        for src in &frontier {
            for e in graph.edges().filter(|e| e.from == *src) {
                if e.confidence.0 < min_confidence || !pass_stale(e) {
                    continue;
                }
                outgoing.push(EdgeAtDepth {
                    edge: e.clone(),
                    depth,
                });
                if visited_out.insert(e.to) {
                    next.push(e.to);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    // Incoming BFS
    let mut frontier = vec![center_id];
    for depth in 1..=max_depth.max(1) {
        let mut next: Vec<coregraph_core::SymbolId> = Vec::new();
        for tgt in &frontier {
            for e in graph.edges().filter(|e| e.to == *tgt) {
                if e.confidence.0 < min_confidence || !pass_stale(e) {
                    continue;
                }
                incoming.push(EdgeAtDepth {
                    edge: e.clone(),
                    depth,
                });
                if visited_in.insert(e.from) {
                    next.push(e.from);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    // Stable ordering: primary by depth, secondary by edge-kind priority.
    // `Resolves` ends up last within a hop because it's the most verbose
    // edge kind in the graph (name-resolution results dominate the edge set)
    // and drowns out `Calls`/`Implements`/`Extends`/`Inherits` in the
    // first page. Structural edges come before `Resolves` so users see
    // `Contains`/`BelongsTo` for context but after semantic edges that
    // actually carry behaviour.
    incoming.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| edge_kind_priority(&a.edge.kind).cmp(&edge_kind_priority(&b.edge.kind)))
    });
    outgoing.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| edge_kind_priority(&a.edge.kind).cmp(&edge_kind_priority(&b.edge.kind)))
    });
    (incoming, outgoing)
}

/// Lower numbers sort earlier in query pages. Semantic relationships
/// (`Calls`, `Implements`, `Extends`, `Inherits`, `Overrides`, `DependsOn`)
/// come first because they answer "what does this symbol *do* in the
/// system"; structural relationships (`Contains`, `BelongsTo`) are
/// framing; `Resolves` is the lowest-signal bulk filler — useful when
/// asked for but never what a user wants to see first.
fn edge_kind_priority(kind: &EdgeKind) -> u8 {
    match kind {
        EdgeKind::Calls => 0,
        EdgeKind::Implements => 1,
        EdgeKind::Extends => 2,
        EdgeKind::Inherits => 3,
        EdgeKind::Overrides => 4,
        EdgeKind::Imports => 5,
        EdgeKind::References => 6,
        EdgeKind::DependsOn => 7,
        EdgeKind::TypeOf => 8,
        EdgeKind::GenericParam => 9,
        EdgeKind::Configures => 10,
        EdgeKind::EnumValueMatch => 11,
        EdgeKind::ApiPathMatch => 12,
        EdgeKind::StringMatch => 13,
        EdgeKind::Contains => 14,
        EdgeKind::BelongsTo => 15,
        // Documentation framing sits just above the bulk Resolves filler.
        EdgeKind::Documents => 16,
        EdgeKind::Mentions => 17,
        EdgeKind::DescribedIn => 18,
        EdgeKind::Resolves => 19,
    }
}

fn edge_tag(kind: &EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Resolves => "resolves",
        EdgeKind::Calls => "calls",
        EdgeKind::Implements => "implements",
        EdgeKind::Extends => "extends",
        EdgeKind::Inherits => "inherits",
        EdgeKind::Overrides => "overrides",
        EdgeKind::References => "references",
        EdgeKind::Imports => "imports",
        EdgeKind::TypeOf => "type-of",
        EdgeKind::GenericParam => "generic-param",
        EdgeKind::StringMatch => "string-match",
        EdgeKind::EnumValueMatch => "enum-value-match",
        EdgeKind::ApiPathMatch => "api-path-match",
        EdgeKind::Configures => "configures",
        EdgeKind::Contains => "contains",
        EdgeKind::BelongsTo => "belongs-to",
        EdgeKind::DependsOn => "depends-on",
        EdgeKind::Documents => "documents",
        EdgeKind::Mentions => "mentions",
        EdgeKind::DescribedIn => "described-in",
    }
}

/// Final rendered output exposed to callers.
///
/// Pagination state (page index, total pages, truncation flag) travels
/// inside `body` itself — either embedded in human output as the footer
/// line, or as JSON fields — so callers don't need them as separate
/// struct fields. If a future caller does need pagination in-process,
/// return a richer type then instead of preserving unused fields now.
pub struct Rendered {
    pub body: String,
}

#[allow(clippy::too_many_arguments)]
pub fn render_symbol(
    graph: &SymbolGraph,
    center: &SymbolNode,
    incoming_all: &[EdgeAtDepth],
    outgoing_all: &[EdgeAtDepth],
    format: OutputFormat,
    color: ColorMode,
    page: usize,
    page_size: usize,
    token_budget: usize,
) -> Rendered {
    let use_color = match color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            matches!(format, OutputFormat::Human)
                && std::io::IsTerminal::is_terminal(&std::io::stdout())
        }
    };

    let actual_budget = effective_budget(token_budget);
    let zones = BudgetZones::for_total(actual_budget);

    // Pagination: slice the edge lists by page/offset (page_size hard cap).
    let total_edges = incoming_all.len() + outgoing_all.len();
    let total_pages = if page_size == 0 {
        1
    } else {
        total_edges.div_ceil(page_size).max(1)
    };
    let start = page * page_size;
    let end = (start + page_size).min(total_edges);

    // Concatenate outgoing first, then incoming, for deterministic paging.
    let mut flat: Vec<(&EdgeAtDepth, bool)> = outgoing_all.iter().map(|e| (e, true)).collect();
    flat.extend(incoming_all.iter().map(|e| (e, false)));
    let empty: &[(&EdgeAtDepth, bool)] = &[];
    let page_slice: &[(&EdgeAtDepth, bool)] = if start < flat.len() {
        &flat[start..end.min(flat.len())]
    } else {
        empty
    };

    let mut page_out: Vec<&EdgeAtDepth> = Vec::new();
    let mut page_in: Vec<&EdgeAtDepth> = Vec::new();
    for (edge, is_out) in page_slice {
        if *is_out {
            page_out.push(edge);
        } else {
            page_in.push(edge);
        }
    }

    let next_cursor = if end < total_edges {
        Some(Cursor {
            page: page + 1,
            offset: end,
        })
    } else {
        None
    };

    let trust = summarize_path_trust(incoming_all, outgoing_all);

    let (body, truncated) = match format {
        OutputFormat::Human => render_human(
            graph,
            center,
            &page_in,
            &page_out,
            page,
            total_pages,
            total_edges,
            actual_budget,
            &zones,
            use_color,
            &trust,
        ),
        OutputFormat::Llm => render_llm(
            graph,
            center,
            &page_in,
            &page_out,
            page,
            total_pages,
            actual_budget,
            &zones,
            next_cursor.as_ref(),
            &trust,
        ),
        OutputFormat::Json => (
            render_json(
                graph,
                center,
                &page_in,
                &page_out,
                page,
                page_size,
                total_pages,
                total_edges,
                token_budget,
                actual_budget,
                next_cursor.as_ref(),
                &trust,
            ),
            false,
        ),
    };
    // `page`, `total_pages`, and `truncated` are embedded in `body`
    // (footer line for human, JSON keys for json/llm) so we drop them
    // from the returned struct rather than carrying unused state.
    let _ = (page, total_pages, truncated);
    Rendered { body }
}

fn node_short(n: &SymbolNode) -> String {
    format!("{} [{:?}] @ {}", n.name, n.kind, n.file.display())
}

/// ANSI helpers.
fn ansi(use_color: bool, code: &str, s: &str) -> String {
    if use_color {
        format!("\x1b[{}m{}\x1b[0m", code, s)
    } else {
        s.to_string()
    }
}
fn green(c: bool, s: &str) -> String {
    ansi(c, "32", s)
}
fn yellow(c: bool, s: &str) -> String {
    ansi(c, "33", s)
}
fn cyan(c: bool, s: &str) -> String {
    ansi(c, "36", s)
}
fn dim(c: bool, s: &str) -> String {
    ansi(c, "2", s)
}

#[allow(clippy::too_many_arguments)]
fn render_human(
    graph: &SymbolGraph,
    center: &SymbolNode,
    incoming: &[&EdgeAtDepth],
    outgoing: &[&EdgeAtDepth],
    page: usize,
    total_pages: usize,
    total_edges: usize,
    budget: usize,
    zones: &BudgetZones,
    color: bool,
    trust: &PathTrust,
) -> (String, bool) {
    let mut out = String::new();
    let mut used = 0usize;
    let mut truncated = false;

    let header = format!(
        "── query: {} ──────────────────────────────────\n\n",
        cyan(color, &center.name)
    );
    used += estimate_tokens(&header).min(zones.header);
    out.push_str(&header);

    let pkg = crate::context::package_of(&center.file);
    let gen_marker = if crate::context::is_generated(&center.file) {
        let gen_name =
            crate::context::detect_generator(&center.file).unwrap_or_else(|| "unknown".into());
        format!(
            "  {} GENERATED (generator: {})\n",
            yellow(color, "⚠"),
            gen_name
        )
    } else {
        String::new()
    };
    let center_block = format!(
        "{} {} [{}:{}]\n  kind: {:?} | package: {}\n{}\n",
        green(color, "✓"),
        center.name,
        center.file.display(),
        header_line(&center.file, center.span_start),
        center.kind,
        pkg,
        gen_marker,
    );
    used += estimate_tokens(&center_block);
    out.push_str(&center_block);

    let mut spent_hop1 = 0usize;
    let mut spent_rest = 0usize;
    let mut emit_edge = |line: &str,
                         depth: usize,
                         out: &mut String,
                         used: &mut usize,
                         truncated: &mut bool|
     -> bool {
        let tk = estimate_tokens(line);
        let (spent, cap) = if depth == 1 {
            (&mut spent_hop1, zones.hop1)
        } else {
            (&mut spent_rest, zones.hop_rest)
        };
        if *spent + tk > cap || *used + tk > budget {
            *truncated = true;
            return false;
        }
        *spent += tk;
        *used += tk;
        out.push_str(line);
        true
    };

    if !outgoing.is_empty() {
        let section = format!("  Outgoing ({}):\n", outgoing.len());
        out.push_str(&section);
        used += estimate_tokens(&section);
        for (i, e) in outgoing.iter().enumerate() {
            let to = graph
                .get_node(e.edge.to)
                .map(node_short)
                .unwrap_or_else(|| format!("(id={:?})", e.edge.to));
            let prefix = if i + 1 == outgoing.len() {
                "  └──"
            } else {
                "  ├──"
            };
            let depth_tag = if e.depth > 1 {
                dim(color, &format!(" (hop={})", e.depth))
            } else {
                String::new()
            };
            let line = format!(
                "{} {} → {}{}      [{:.2}] {}\n",
                prefix,
                edge_tag(&e.edge.kind),
                to,
                depth_tag,
                e.edge.confidence.0,
                green(color, "✓")
            );
            if !emit_edge(&line, e.depth, &mut out, &mut used, &mut truncated) {
                break;
            }
        }
        out.push('\n');
    }

    if !incoming.is_empty() {
        let section = format!("  Incoming ({}):\n", incoming.len());
        out.push_str(&section);
        used += estimate_tokens(&section);
        for (i, e) in incoming.iter().enumerate() {
            let from = graph
                .get_node(e.edge.from)
                .map(node_short)
                .unwrap_or_else(|| format!("(id={:?})", e.edge.from));
            let prefix = if i + 1 == incoming.len() {
                "  └──"
            } else {
                "  ├──"
            };
            let depth_tag = if e.depth > 1 {
                dim(color, &format!(" (hop={})", e.depth))
            } else {
                String::new()
            };
            let line = format!(
                "{} {} ← {}{}      [{:.2}] {}\n",
                prefix,
                edge_tag(&e.edge.kind),
                from,
                depth_tag,
                e.edge.confidence.0,
                green(color, "✓")
            );
            if !emit_edge(&line, e.depth, &mut out, &mut used, &mut truncated) {
                break;
            }
        }
    }

    if truncated {
        let warn = format!(
            "\n{} budget ({}/{}) exceeded — re-run with --page-size or --token-budget\n",
            yellow(color, "⚠"),
            used,
            budget
        );
        if used + estimate_tokens(&warn) <= budget + zones.warnings {
            out.push_str(&warn);
        }
    }

    // Path trust line (§11.5)
    let trust_line = match trust {
        PathTrust::Verified => format!("  {} trust: all paths verified\n", green(color, "✓")),
        PathTrust::Broken { at_hop, stale_file } => format!(
            "  {} trust: broken at hop {} — {}\n",
            yellow(color, "⚠"),
            at_hop,
            stale_file
        ),
        PathTrust::Partial {
            verified_paths,
            broken_paths,
        } => format!(
            "  {} trust: partial ({} verified, {} broken)\n",
            yellow(color, "⚠"),
            verified_paths,
            broken_paths
        ),
    };
    out.push_str(&trust_line);

    // Interactive footer (§11.1)
    let footer = format!(
        "\n── page {}/{} | {} edges total | budget: {}/{} tokens ──\n   [n]ext page | [e]xpand <id> | [f]ilter --edge-kind | [q]uit\n",
        page + 1,
        total_pages,
        total_edges,
        used,
        budget
    );
    out.push_str(&footer);
    (out, truncated)
}

#[allow(clippy::too_many_arguments)]
fn render_llm(
    graph: &SymbolGraph,
    center: &SymbolNode,
    incoming: &[&EdgeAtDepth],
    outgoing: &[&EdgeAtDepth],
    page: usize,
    total_pages: usize,
    budget: usize,
    zones: &BudgetZones,
    next_cursor: Option<&Cursor>,
    trust: &PathTrust,
) -> (String, bool) {
    let mut out = String::new();
    let mut used = 0usize;
    let mut truncated = false;

    let pkg = crate::context::package_of(&center.file);
    let generated_line = if crate::context::is_generated(&center.file) {
        let gen =
            crate::context::detect_generator(&center.file).unwrap_or_else(|| "unknown".into());
        format!("- ⚠ generated: true\n- generator: {}\n", gen)
    } else {
        String::new()
    };
    let header = format!(
        "## Symbol: {}\n- file: {}:{}\n- kind: {:?}\n- package: {}\n{}\n",
        center.name,
        center.file.display(),
        header_line(&center.file, center.span_start),
        center.kind,
        pkg,
        generated_line
    );
    used += estimate_tokens(&header);
    out.push_str(&header);

    let mut spent_hop1 = 0usize;
    let mut spent_rest = 0usize;

    if !outgoing.is_empty() {
        out.push_str("### Outgoing edges\n| target | edge | depth | confidence | status |\n|---|---|---|---|---|\n");
        used += 90;
        for e in outgoing {
            let target = graph
                .get_node(e.edge.to)
                .map(|n| n.name.clone())
                .unwrap_or_default();
            let line = format!(
                "| {} | {} | {} | {:.2} | {} |\n",
                target,
                edge_tag(&e.edge.kind),
                e.depth,
                e.edge.confidence.0,
                format!("{:?}", e.edge.origin).to_lowercase()
            );
            let tk = estimate_tokens(&line);
            let (spent, cap) = if e.depth == 1 {
                (&mut spent_hop1, zones.hop1)
            } else {
                (&mut spent_rest, zones.hop_rest)
            };
            if *spent + tk > cap || used + tk > budget {
                truncated = true;
                break;
            }
            *spent += tk;
            used += tk;
            out.push_str(&line);
        }
        out.push('\n');
    }

    if !incoming.is_empty() && !truncated {
        out.push_str("### Incoming edges\n| source | edge | depth | confidence | status |\n|---|---|---|---|---|\n");
        used += 90;
        for e in incoming {
            let source = graph
                .get_node(e.edge.from)
                .map(|n| n.name.clone())
                .unwrap_or_default();
            let line = format!(
                "| {} | {} | {} | {:.2} | {} |\n",
                source,
                edge_tag(&e.edge.kind),
                e.depth,
                e.edge.confidence.0,
                format!("{:?}", e.edge.origin).to_lowercase()
            );
            let tk = estimate_tokens(&line);
            let (spent, cap) = if e.depth == 1 {
                (&mut spent_hop1, zones.hop1)
            } else {
                (&mut spent_rest, zones.hop_rest)
            };
            if *spent + tk > cap || used + tk > budget {
                truncated = true;
                break;
            }
            *spent += tk;
            used += tk;
            out.push_str(&line);
        }
        out.push('\n');
    }

    if let Some(c) = next_cursor {
        out.push_str(&format!(
            "<!-- coregraph:pagination page={} total_pages={} next=\"coregraph query {} --cursor {}\" -->\n",
            c.page,
            total_pages,
            center.name,
            encode_cursor(c),
        ));
    }
    // §11.5 PathTrust annotation
    let trust_str = match trust {
        PathTrust::Verified => "verified".to_string(),
        PathTrust::Broken { at_hop, stale_file } => {
            format!("broken at_hop={} stale_file={}", at_hop, stale_file)
        }
        PathTrust::Partial {
            verified_paths,
            broken_paths,
        } => format!(
            "partial verified={} broken={}",
            verified_paths, broken_paths
        ),
    };
    out.push_str(&format!("<!-- coregraph:path_trust {} -->\n", trust_str));

    out.push_str(&format!(
        "<!-- coregraph:budget used={} total={} remaining={} estimation=byte_approximation truncated={} -->\n",
        used,
        budget,
        budget.saturating_sub(used),
        truncated,
    ));
    out.push_str(&format!(
        "<!-- coregraph:actions expand=\"coregraph query {} --expand <id>\" source=\"coregraph inspect {}:{}\" deeper=\"coregraph query {} --depth 5\" -->\n",
        center.name,
        center.file.display(),
        header_line(&center.file, center.span_start),
        center.name,
    ));
    _ = page; // page is embedded in cursor
    (out, truncated)
}

#[allow(clippy::too_many_arguments)]
fn render_json(
    graph: &SymbolGraph,
    center: &SymbolNode,
    incoming: &[&EdgeAtDepth],
    outgoing: &[&EdgeAtDepth],
    page: usize,
    page_size: usize,
    total_pages: usize,
    total_edges: usize,
    advertised_budget: usize,
    actual_budget: usize,
    next_cursor: Option<&Cursor>,
    trust: &PathTrust,
) -> String {
    let edge_json = |e: &EdgeAtDepth, dir: &str| -> serde_json::Value {
        let other = match dir {
            "outgoing" => e.edge.to,
            _ => e.edge.from,
        };
        let other_name = graph
            .get_node(other)
            .map(|n| n.name.clone())
            .unwrap_or_default();
        serde_json::json!({
            "direction": dir,
            "kind": edge_tag(&e.edge.kind),
            "depth": e.depth,
            "other_id": other,
            "other_name": other_name,
            "confidence": e.edge.confidence.0,
            // Legacy field retained for compatibility (mirrors `origin`).
            "trust": format!("{:?}", e.edge.origin),
            "origin": format!("{:?}", e.edge.origin),
            "trust_model": format!("{:?}", e.edge.trust_model()),
            "stale_evidence_count": e.edge.stale_evidence_count,
            "current_confidence": e.edge.current_confidence(),
        })
    };
    // Emit edges while respecting the effective token budget (one-pass truncation).
    let mut edges: Vec<serde_json::Value> = Vec::new();
    let mut used: usize = 0;
    let mut truncated = false;
    // Header/center cost estimate (≈ center json + scaffolding)
    used += estimate_tokens(&serde_json::to_string(&center.name).unwrap_or_default()) + 40;
    let tagged = outgoing
        .iter()
        .map(|e| (*e, "outgoing"))
        .chain(incoming.iter().map(|e| (*e, "incoming")));
    for (e, dir) in tagged {
        let j = edge_json(e, dir);
        let cost = estimate_tokens(&j.to_string());
        if used + cost > actual_budget {
            truncated = true;
            break;
        }
        used += cost;
        edges.push(j);
    }

    let root = serde_json::json!({
        "query": center.name,
        "center": {
            "id": center.id,
            "name": center.name,
            "kind": format!("{:?}", center.kind),
            "file": center.file.display().to_string(),
            "span_start": center.span_start,
            "span_end": center.span_end,
            "context": crate::context::node_context(center),
        },
        "edges": edges,
        "pagination": {
            "page": page,
            "total_pages": total_pages,
            "total_edges": total_edges,
            "page_size": page_size,
            "next_cursor": next_cursor.map(encode_cursor),
        },
        "budget": {
            "used_tokens": used,
            "total_tokens": actual_budget,
            "remaining_tokens": actual_budget.saturating_sub(used),
            "advertised_total": advertised_budget,
            "estimation_method": "byte_approximation",
            "truncated": truncated,
        },
        "path_trust": trust,
    });
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_safety_margin() {
        assert_eq!(effective_budget(8000), 5600);
        assert_eq!(effective_budget(1000), 700);
    }

    #[test]
    fn estimate_tokens_english() {
        assert_eq!(estimate_tokens(""), 1);
        assert_eq!(estimate_tokens("abcd"), 2);
    }

    #[test]
    fn line_of_byte_counts_newlines() {
        let s = b"line1\nline2\nXfoo"; // newlines at byte 5 and 11
        assert_eq!(line_of_byte(s, 0), 1, "offset 0 is line 1");
        assert_eq!(line_of_byte(s, 12), 3, "start of 3rd line");
        assert_eq!(line_of_byte(s, 9999), 3, "past EOF clamps to last line");
    }

    #[test]
    fn cursor_roundtrip() {
        let c = Cursor {
            page: 2,
            offset: 50,
        };
        let encoded = encode_cursor(&c);
        let decoded = decode_cursor(&encoded).unwrap();
        assert_eq!(decoded.page, 2);
        assert_eq!(decoded.offset, 50);
    }

    #[test]
    fn cursor_rejects_garbage() {
        assert!(decode_cursor("!!!not-a-cursor!!!").is_none());
    }

    #[test]
    fn budget_zones_sum_under_whole() {
        let z = BudgetZones::for_total(10_000);
        let sum = z.header + z.hop1 + z.hop_rest + z.warnings;
        // Sum should stay below the whole (header 2.5 + 30 + 43.75 + 6.25 ≈ 82.5%).
        assert!(sum <= 10_000);
        assert!(sum >= 7_000);
    }

    #[test]
    fn bfs_depth_1_direct_edges_only() {
        use coregraph_core::{
            edge::{AnalysisOrigin, Confidence},
            SymbolId, SymbolKind, SymbolNode,
        };
        let mut g = SymbolGraph::new();
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a",
            "f.rs",
            0,
            1,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b",
            "f.rs",
            0,
            1,
        ));
        let c = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "c",
            "f.rs",
            0,
            1,
        ));
        g.insert_edge(DirectEdge::new(
            a,
            b,
            EdgeKind::Calls,
            AnalysisOrigin::NameResolved,
            Confidence::new(0.9),
            "f.rs",
        ));
        g.insert_edge(DirectEdge::new(
            b,
            c,
            EdgeKind::Calls,
            AnalysisOrigin::NameResolved,
            Confidence::new(0.9),
            "f.rs",
        ));

        let (_, out1) = bfs_edges_filtered(&g, a, 1, 0.0, true);
        assert_eq!(out1.len(), 1);
        let (_, out2) = bfs_edges_filtered(&g, a, 2, 0.0, true);
        assert_eq!(out2.len(), 2);
    }

    #[test]
    fn pick_center_prefers_exported_definition() {
        use coregraph_core::{SymbolId, SymbolKind, SymbolNode, Visibility};
        // A test-local helper (Unknown visibility) precedes the public API
        // definition in the match list; the exported one must win as center.
        let local = SymbolNode::new(
            SymbolId(1),
            SymbolKind::Function,
            "createStore",
            "t.tsx",
            0,
            1,
        )
        .with_visibility(Visibility::Unknown);
        let public = SymbolNode::new(
            SymbolId(2),
            SymbolKind::Function,
            "createStore",
            "vanilla.ts",
            0,
            1,
        )
        .with_visibility(Visibility::Public);
        let matches = vec![&local, &public];
        assert_eq!(
            pick_center_index(&matches),
            1,
            "exported definition chosen as center"
        );
    }

    #[test]
    fn pick_center_falls_back_to_first_when_none_exported() {
        use coregraph_core::{SymbolId, SymbolKind, SymbolNode, Visibility};
        let a = SymbolNode::new(SymbolId(1), SymbolKind::Function, "foo", "a.rs", 0, 1)
            .with_visibility(Visibility::Unknown);
        let b = SymbolNode::new(SymbolId(2), SymbolKind::Function, "foo", "b.rs", 0, 1)
            .with_visibility(Visibility::Unknown);
        let matches = vec![&a, &b];
        assert_eq!(pick_center_index(&matches), 0);
    }

    #[test]
    fn multi_def_note_lists_other_same_name_definitions() {
        use coregraph_core::{SymbolId, SymbolKind, SymbolNode, Visibility};
        let center = SymbolNode::new(
            SymbolId(1),
            SymbolKind::Function,
            "createStore",
            "vanilla.ts",
            0,
            1,
        )
        .with_visibility(Visibility::Public);
        let other = SymbolNode::new(
            SymbolId(2),
            SymbolKind::Function,
            "createStore",
            "persist.test.tsx",
            0,
            1,
        )
        .with_visibility(Visibility::Unknown);
        // A fuzzy match with a different name must NOT be listed.
        let fuzzy = SymbolNode::new(
            SymbolId(3),
            SymbolKind::Function,
            "createStoreImpl",
            "vanilla.ts",
            0,
            1,
        )
        .with_visibility(Visibility::Unknown);
        let matches = vec![&center, &other, &fuzzy];
        let note = multi_def_note(&matches, 0).expect("note expected for multiple definitions");
        assert!(
            note.contains("persist.test.tsx"),
            "lists the other same-name definition: {note}"
        );
        assert!(
            !note.contains("createStoreImpl"),
            "excludes different-name fuzzy match: {note}"
        );
    }

    #[test]
    fn multi_def_note_absent_for_unique_definition() {
        use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
        let only = SymbolNode::new(SymbolId(1), SymbolKind::Function, "foo", "a.rs", 0, 1);
        let matches = vec![&only];
        assert!(
            multi_def_note(&matches, 0).is_none(),
            "no note when the name is unique"
        );
    }

    #[test]
    fn aggregated_bfs_unions_callers_across_same_name_nodes() {
        use coregraph_core::{
            edge::{AnalysisOrigin, Confidence},
            SymbolId, SymbolKind, SymbolNode,
        };
        // Two overloads of `foo` (e.g. method overloads, or a module fn and a
        // method of the same name), each with a distinct caller. A query
        // centred on one overload misses the other's callers — aggregation
        // across the same-name group must surface both.
        let mut g = SymbolGraph::new();
        let foo1 = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "foo",
            "a.rs",
            0,
            1,
        ));
        let foo2 = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "foo",
            "b.rs",
            0,
            1,
        ));
        let c1 = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "caller_one",
            "a.rs",
            0,
            1,
        ));
        let c2 = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "caller_two",
            "b.rs",
            0,
            1,
        ));
        g.insert_edge(DirectEdge::new(
            c1,
            foo1,
            EdgeKind::Calls,
            AnalysisOrigin::NameResolved,
            Confidence::new(0.9),
            "a.rs",
        ));
        g.insert_edge(DirectEdge::new(
            c2,
            foo2,
            EdgeKind::Calls,
            AnalysisOrigin::NameResolved,
            Confidence::new(0.9),
            "b.rs",
        ));

        let (in_single, _) = bfs_edges_filtered(&g, foo1, 1, 0.0, true);
        assert_eq!(
            in_single.len(),
            1,
            "single-overload query sees only its own caller"
        );

        let (in_agg, _) = bfs_edges_aggregated(&g, &[foo1, foo2], 1, 0.0, true);
        assert_eq!(
            in_agg.len(),
            2,
            "aggregation unions callers of both overloads"
        );
    }

    #[test]
    fn aggregated_bfs_dedups_shared_edges() {
        use coregraph_core::{
            edge::{AnalysisOrigin, Confidence},
            SymbolId, SymbolKind, SymbolNode,
        };
        // An edge reachable from more than one center (here a self-edge between
        // the two same-name overloads) must appear once, not once per center.
        let mut g = SymbolGraph::new();
        let foo1 = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "foo",
            "a.rs",
            0,
            1,
        ));
        let foo2 = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "foo",
            "a.rs",
            2,
            3,
        ));
        g.insert_edge(DirectEdge::new(
            foo1,
            foo2,
            EdgeKind::Calls,
            AnalysisOrigin::NameResolved,
            Confidence::new(0.9),
            "a.rs",
        ));
        let (inc, out) = bfs_edges_aggregated(&g, &[foo1, foo2], 1, 0.0, true);
        // The foo1->foo2 edge is outgoing for foo1 and incoming for foo2; it
        // appears once in each direction's list, never duplicated within one.
        assert_eq!(out.len(), 1, "shared edge not duplicated in outgoing");
        assert_eq!(inc.len(), 1, "shared edge not duplicated in incoming");
    }

    #[test]
    fn bfs_min_confidence_filter() {
        use coregraph_core::{
            edge::{AnalysisOrigin, Confidence},
            SymbolId, SymbolKind, SymbolNode,
        };
        let mut g = SymbolGraph::new();
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a",
            "f.rs",
            0,
            1,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b",
            "f.rs",
            0,
            1,
        ));
        g.insert_edge(DirectEdge::new(
            a,
            b,
            EdgeKind::Calls,
            AnalysisOrigin::NameResolved,
            Confidence::new(0.5),
            "f.rs",
        ));
        let (_, out) = bfs_edges_filtered(&g, a, 3, 0.7, true);
        assert!(out.is_empty(), "0.5 confidence edge filtered at min 0.7");
    }
}
