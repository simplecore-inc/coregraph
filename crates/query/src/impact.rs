use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolKind, SymbolNode};
use coregraph_graph::SymbolGraph;

#[derive(Debug, Clone)]
pub struct ImpactResult {
    pub seed: SymbolId,
    pub reachable: Vec<SymbolNode>,
    pub edges: Vec<DirectEdge>,
    pub depth_reached: usize,
}

impl ImpactResult {
    pub fn node_count(&self) -> usize {
        self.reachable.len()
    }
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// Traverse the reverse dependents cone: every symbol that reaches the seed
/// through incoming impact-bearing edges within `max_depth` hops.
///
/// "Impact of X" = who breaks if X changes — X's direct callers/users, their
/// callers, and so on. Only incoming edges are followed: an earlier version
/// walked edges in BOTH directions, which turned `reachable` into a generic
/// connectivity count (a depth-5 sweep from a 3-caller helper reached 74% of
/// this repo's graph via shared callees), not an impact measure. What X itself
/// depends on (outgoing) does not break when X changes.
pub fn compute_impact(graph: &SymbolGraph, seed_id: SymbolId, max_depth: usize) -> ImpactResult {
    let mut visited: HashSet<SymbolId> = HashSet::new();
    let mut queue: VecDeque<(SymbolId, usize)> = VecDeque::new();
    let mut reachable: Vec<SymbolNode> = vec![];
    let mut traversed_edges: Vec<DirectEdge> = vec![];
    let mut depth_reached = 0;

    visited.insert(seed_id);
    queue.push_back((seed_id, 0));

    while let Some((current_id, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        // O(degree) over edges incident to `current_id`, not O(E) over the
        // whole graph — the previous full scan made each query O(V·E).
        for (next, edge) in graph.incident_edges(current_id) {
            // Dependents only: keep edges that point AT the current node.
            if edge.to != current_id {
                continue;
            }
            // Skip structural / decorative edges (Contains, BelongsTo,
            // StringMatch, …). They were blowing up impact cones — a
            // File node's `Contains` edge reaches every symbol in the
            // file in one hop, pushing depth-3 impact to 50% of the
            // whole graph. Changes to `build_graph` don't actually
            // affect siblings in the same file via `Contains`.
            if !is_impact_bearing(&edge.kind) {
                continue;
            }
            // Skip container nodes (File / doc comments): a file-level
            // Resolves edge is an attribution fallback, not a dependent,
            // and traversing through File nodes fans the cone file-wide.
            let Some(node) = graph.get_node(next) else {
                continue;
            };
            if !is_impact_node(&node.kind) {
                continue;
            }
            // Record the edge only on first visit of `next`, so `edge_count`
            // reflects the impact spanning tree rather than every incident
            // edge seen from both endpoints (which double-counted).
            if visited.insert(next) {
                depth_reached = depth_reached.max(depth + 1);
                reachable.push(node.clone());
                traversed_edges.push(edge.clone());
                queue.push_back((next, depth + 1));
            }
        }
    }

    ImpactResult {
        seed: seed_id,
        reachable,
        edges: traversed_edges,
        depth_reached,
    }
}

/// Edge kinds that carry semantic impact: a change to the source node
/// can plausibly break callers / consumers traversed via this kind.
/// Structural (`Contains`, `BelongsTo`), matching (`StringMatch`,
/// `ApiPathMatch`, `EnumValueMatch`), decorative (`Configures`,
/// `TypeOf`, `GenericParam`), and documentation (`Documents`, `Mentions`,
/// `DescribedIn`) edges are excluded — they describe layout, co-occurrence, or
/// annotation, not causal reachability. A doc node is never code-that-breaks:
/// editing a doc (or the symbol it mentions / describes) does not inflate a code
/// blast radius. Doc staleness when a referenced symbol changes is a drift
/// concern, not impact; "which docs describe X" is a reverse-edge query.
fn is_impact_bearing(kind: &EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Calls
            | EdgeKind::Implements
            | EdgeKind::Extends
            | EdgeKind::Inherits
            | EdgeKind::Overrides
            | EdgeKind::Resolves
            | EdgeKind::References
            | EdgeKind::DependsOn
            | EdgeKind::Imports
    )
}

/// Node kinds that count as dependents in an impact/risk cone. File and
/// doc-comment containers are excluded: a file-level `Resolves` edge is the
/// syntactic fallback's coarse attribution (the real referrer could not be
/// pinned to a symbol), and a `DocComment` mentioning a name is not code that
/// breaks — counting either inflates `reachable`/`caller_count` and lets the
/// traversal jump file-wide through container hubs.
fn is_impact_node(kind: &SymbolKind) -> bool {
    !matches!(
        kind,
        SymbolKind::File | SymbolKind::DocComment | SymbolKind::DocSection
    )
}

/// Number of incoming impact-bearing edges from dependent-eligible nodes —
/// the seed-selection signal for `pick_impact_seed`.
fn incoming_impact_degree(graph: &SymbolGraph, id: SymbolId) -> usize {
    graph
        .incident_edges(id)
        .into_iter()
        .filter(|(src, e)| {
            e.to == id
                && is_impact_bearing(&e.kind)
                && graph.get_node(*src).is_some_and(|n| is_impact_node(&n.kind))
        })
        .count()
}

/// Pick the impact seed among same-name candidates: exact name matches first
/// (substring fallback), then the definition with the most incoming
/// impact-bearing edges, first match on ties.
///
/// Same-name twins are common (constructors, trait methods, per-type helpers)
/// and "first node wins" is arbitrary: seeding the analysis on an uncalled
/// twin reports zero dependents while the definition users actually call has
/// a real cone. Preferring the most-referenced definition answers the
/// question a name-keyed impact query is actually asking.
pub fn pick_impact_seed<'g>(graph: &'g SymbolGraph, name: &str) -> Option<&'g SymbolNode> {
    let exact: Vec<&SymbolNode> = graph.nodes().filter(|n| n.name == name).collect();
    let candidates: Vec<&SymbolNode> = if exact.is_empty() {
        graph.nodes().filter(|n| n.name.contains(name)).collect()
    } else {
        exact
    };
    candidates
        .into_iter()
        .enumerate()
        .max_by_key(|(i, n)| {
            (
                incoming_impact_degree(graph, n.id),
                std::cmp::Reverse(*i), // ties: keep the first candidate
            )
        })
        .map(|(_, n)| n)
}

// ──────────────────────────────────────────────────────────────────────────
// Impact Risk Scoring (docs/graph-model.md §5)
// ──────────────────────────────────────────────────────────────────────────

/// Blast radius classification per docs §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BlastRadius {
    Low,
    Medium,
    High,
    Critical,
}

/// Narrative risk bucket derived from the composite 0–1 score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn from_score(score: f64) -> Self {
        if score < 0.4 {
            RiskLevel::Low
        } else if score < 0.6 {
            RiskLevel::Medium
        } else if score < 0.8 {
            RiskLevel::High
        } else {
            RiskLevel::Critical
        }
    }
}

/// A downstream test touched by a change to the seed symbol.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TestInfo {
    pub id: SymbolId,
    pub name: String,
    pub file: String,
    pub distance: usize,
    pub path_confidence: f64,
}

/// Result of `compute_risk`. Four-factor weighted score plus the CoreGraph
/// differentiator: confidence-weighted blast radius.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImpactRisk {
    pub blast_radius: BlastRadius,
    pub risk_score: f64,
    pub risk_level: RiskLevel,
    pub confidence_weighted_impact: f64,
    pub module_count: usize,
    pub caller_count: usize,
    pub visibility_score: f64,
    pub caller_factor: f64,
    pub module_factor: f64,
    pub impact_kind_factor: f64,
    pub affected_tests: Vec<TestInfo>,
}

impl ImpactRisk {
    /// Reference to the blast-radius classification.
    pub fn blast_radius_label(&self) -> &'static str {
        match self.blast_radius {
            BlastRadius::Low => "Low",
            BlastRadius::Medium => "Medium",
            BlastRadius::High => "High",
            BlastRadius::Critical => "Critical",
        }
    }

    pub fn risk_level_label(&self) -> &'static str {
        match self.risk_level {
            RiskLevel::Low => "Low",
            RiskLevel::Medium => "Medium",
            RiskLevel::High => "High",
            RiskLevel::Critical => "Critical",
        }
    }
}

/// Multiply confidence along a BFS path, clamped to [0,1].
pub fn path_confidence(edges: &[DirectEdge]) -> f64 {
    edges
        .iter()
        .map(|e| e.current_confidence())
        .product::<f64>()
        .clamp(0.0, 1.0)
}

/// Compute `ImpactRisk` for `seed_id` by walking the incoming-edge cone up to
/// `max_depth`. Uses `current_confidence()` (origin-based, decayed) so
/// freshly-invalidated evidence dampens the risk contribution.
///
/// The cone applies the same `is_impact_bearing()` edge filter and
/// `is_impact_node()` node filter as `compute_impact`, so `caller_count`,
/// `confidence_weighted_impact`, the module spread, and `affected_tests` count
/// only genuine dependents. Without the filters, structural (`Contains`),
/// documentation (`Documents`), and file-level attribution edges put the
/// seed's own File and DocComment nodes into the "caller" set — a symbol with
/// zero real callers still reported 3 callers and a non-zero risk.
pub fn compute_risk(graph: &SymbolGraph, seed_id: SymbolId, max_depth: usize) -> ImpactRisk {
    let Some(seed) = graph.get_node(seed_id) else {
        return empty_risk();
    };

    // Walk the reverse graph: every dependent that reaches `seed_id` through
    // at most `max_depth` incoming impact-bearing hops (see the function doc).
    // Track the best path confidence to each node and its hop distance.
    let mut best_conf: HashMap<SymbolId, (f64, usize)> = HashMap::new();
    best_conf.insert(seed_id, (1.0, 0));

    let mut queue: VecDeque<(SymbolId, f64, usize)> = VecDeque::new();
    queue.push_back((seed_id, 1.0, 0));

    while let Some((current_id, path_conf, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        // O(degree) incoming edges to `current_id`, not an O(E) scan per node.
        for (caller, edge) in graph.incident_edges(current_id) {
            if edge.to != current_id {
                continue; // keep only incoming edges (reverse cone)
            }
            if !is_impact_bearing(&edge.kind) {
                continue; // Contains/Documents/StringMatch are not callers
            }
            // File / doc-comment containers are not callers (see is_impact_node).
            if !graph
                .get_node(caller)
                .is_some_and(|n| is_impact_node(&n.kind))
            {
                continue;
            }
            let new_conf = (path_conf * edge.current_confidence()).clamp(0.0, 1.0);
            let entry = best_conf.entry(caller).or_insert((0.0, depth + 1));
            if new_conf > entry.0 {
                entry.0 = new_conf;
                entry.1 = depth + 1;
                queue.push_back((caller, new_conf, depth + 1));
            }
        }
    }

    // Remove the seed from the caller cone.
    best_conf.remove(&seed_id);

    let caller_count = best_conf.len();
    let confidence_weighted_impact: f64 = best_conf.values().map(|(c, _)| *c).sum();

    // Module spread: count distinct crates / top-level modules in the cone
    // plus the seed's own module, weighted by the best confidence per module.
    let mut per_module_confidence: HashMap<String, f64> = HashMap::new();
    for (&caller_id, &(conf, _)) in &best_conf {
        if let Some(node) = graph.get_node(caller_id) {
            let module = module_of(&node.file);
            let entry = per_module_confidence.entry(module).or_insert(0.0);
            if conf > *entry {
                *entry = conf;
            }
        }
    }
    let module_count = per_module_confidence.len().max(1);
    let module_weighted_sum: f64 = per_module_confidence.values().sum();

    // Visibility: see `visibility_of` — 1.0 for public-looking symbols, 0.75
    // for ordinary (non-private) functions, 0.4 for private-looking ones.
    // Rust's `fn foo(...)` is implicitly public-to-the-crate; `_leading` is
    // private by convention. Classes, traits, interfaces, and enums default to
    // public.
    let visibility_score = visibility_of(seed);

    // Impact kind: incoming edges that represent potentially breaking
    // relationships (Calls/Implements/Extends/Overrides/Resolves/References/
    // DependsOn) vs softer ones (StringMatch/Configures).
    let incoming_edges: Vec<&DirectEdge> = graph
        .incident_edges(seed_id)
        .into_iter()
        .filter(|(_, e)| e.to == seed_id)
        .map(|(_, e)| e)
        .collect();
    let impact_kind_factor = compute_impact_kind_factor(&incoming_edges);

    // Normalize caller factor: saturate at 20 callers weighted by avg conf.
    let caller_factor = (confidence_weighted_impact / 20.0).clamp(0.0, 1.0);
    // Normalize module factor: saturate at 10 modules.
    let module_factor = (module_weighted_sum / 10.0).clamp(0.0, 1.0);

    // Weighted composite per docs §5 (Visibility 0.20 / callers 0.45 /
    // modules 0.25 / impact-kind 0.10). Visibility is a name-shape heuristic
    // (the weakest signal), so it carries less weight than the measured caller
    // cone (the strongest, most reliable signal) — weighted 0.10 lower than the
    // caller_factor so one bad visibility guess cannot flip the risk level.
    let risk_score = (0.20 * visibility_score
        + 0.45 * caller_factor
        + 0.25 * module_factor
        + 0.10 * impact_kind_factor)
        .clamp(0.0, 1.0);

    let blast_radius = classify_blast_radius(module_count, caller_count);

    // Affected tests: reachable callers (via the reverse cone) whose file
    // path looks like a test file.
    let mut affected_tests: Vec<TestInfo> = best_conf
        .iter()
        .filter_map(|(&caller_id, &(conf, distance))| {
            let node = graph.get_node(caller_id)?;
            // A test is an executable unit. Synthetic / non-executable nodes
            // (string-literal refs like env_ref::/api_path::, config keys, enum
            // variants, type-only declarations) that happen to live in a test
            // path are not tests, so they must never appear here.
            if !matches!(node.kind, SymbolKind::Function | SymbolKind::Method) {
                return None;
            }
            if !is_test_symbol(node) {
                return None;
            }
            Some(TestInfo {
                id: node.id,
                name: node.name.clone(),
                file: node.file.display().to_string(),
                distance,
                path_confidence: conf,
            })
        })
        .collect();
    affected_tests.sort_by(|a, b| {
        b.path_confidence
            .partial_cmp(&a.path_confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    ImpactRisk {
        blast_radius,
        risk_score,
        risk_level: RiskLevel::from_score(risk_score),
        confidence_weighted_impact,
        module_count,
        caller_count,
        visibility_score,
        caller_factor,
        module_factor,
        impact_kind_factor,
        affected_tests,
    }
}

fn empty_risk() -> ImpactRisk {
    ImpactRisk {
        blast_radius: BlastRadius::Low,
        risk_score: 0.0,
        risk_level: RiskLevel::Low,
        confidence_weighted_impact: 0.0,
        module_count: 0,
        caller_count: 0,
        visibility_score: 0.0,
        caller_factor: 0.0,
        module_factor: 0.0,
        impact_kind_factor: 0.0,
        affected_tests: Vec::new(),
    }
}

fn classify_blast_radius(modules: usize, callers: usize) -> BlastRadius {
    // docs §5 thresholds
    if modules > 10 || callers > 50 {
        BlastRadius::Critical
    } else if modules > 5 || callers > 20 {
        BlastRadius::High
    } else if modules >= 3 || callers >= 6 {
        BlastRadius::Medium
    } else {
        BlastRadius::Low
    }
}

fn module_of(path: &Path) -> String {
    let s = path.to_string_lossy();
    let segs: Vec<&str> = s.split('/').collect();
    for (i, seg) in segs.iter().enumerate() {
        if *seg == "crates" && i + 1 < segs.len() {
            return segs[i + 1].to_string();
        }
        if *seg == "src" && i >= 1 {
            return segs[i - 1].to_string();
        }
    }
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        return stem.to_string();
    }
    "unknown".to_string()
}

fn visibility_of(node: &SymbolNode) -> f64 {
    // Prefer the extractor-declared visibility (the root-cause signal: an actual
    // `pub` / `export` / `public` keyword) over the name-shape guess below.
    match node.visibility {
        coregraph_core::Visibility::Public => return 1.0,
        coregraph_core::Visibility::Private => return 0.4,
        coregraph_core::Visibility::Unknown => {} // fall back to the name heuristic
    }
    // `_leading` identifiers are conventionally private in Rust/Python.
    // Names that look like system-internal helpers get 0.4.
    let name = &node.name;
    let looks_private = name.starts_with('_') || name.starts_with("__") || name.contains("::impl#");
    let kind_bonus = matches!(
        node.kind,
        SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::Trait
            | SymbolKind::Enum
            | SymbolKind::Struct
            | SymbolKind::TypeAlias
            | SymbolKind::Module
            | SymbolKind::Namespace
    );
    match (looks_private, kind_bonus) {
        (false, true) => 1.0,
        (false, false) => 0.75,
        (true, _) => 0.4,
    }
}

fn compute_impact_kind_factor(edges: &[&DirectEdge]) -> f64 {
    if edges.is_empty() {
        return 0.0;
    }
    let mut breaking_weight = 0.0;
    let mut total_weight = 0.0;
    for e in edges {
        let conf = e.current_confidence();
        total_weight += conf;
        let is_breaking = matches!(
            e.kind,
            EdgeKind::Calls
                | EdgeKind::Implements
                | EdgeKind::Extends
                | EdgeKind::Overrides
                | EdgeKind::Resolves
                | EdgeKind::References
                | EdgeKind::DependsOn
        );
        if is_breaking {
            breaking_weight += conf;
        }
    }
    if total_weight < f64::EPSILON {
        return 0.0;
    }
    (breaking_weight / total_weight).clamp(0.0, 1.0)
}

/// Return all symbols whose span (`span_start..span_end`) encloses the byte
/// offset of `line` (0-based) in `file`. The file is read from disk on every
/// call; callers that already hold the source text should use
/// `symbols_enclosing_line_in_source` directly.
///
/// Returns an empty `Vec` when the file cannot be read or no symbol's span
/// contains the byte offset of `line`.
pub fn symbols_enclosing_line<'a>(
    graph: &'a SymbolGraph,
    file: &'a Path,
    line: u32,
) -> Vec<&'a coregraph_core::SymbolNode> {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let byte_offset = match line_to_byte_offset(&source, line) {
        Some(off) => off,
        None => return Vec::new(),
    };
    graph
        .nodes_in_file(file)
        .filter(|n| n.span_start <= byte_offset && byte_offset < n.span_end)
        .collect()
}

/// Convert a 0-based line number to the byte offset of the first byte on
/// that line. Returns `None` if `line` is beyond the end of the source.
fn line_to_byte_offset(source: &str, line: u32) -> Option<u32> {
    if line == 0 {
        return Some(0);
    }
    let mut current_line: u32 = 0;
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            current_line += 1;
            if current_line == line {
                return Some((i + 1) as u32);
            }
        }
    }
    None
}

/// Whether `node` is test code: either its file is on a test path, or the
/// extractor flagged it (`is_test`) from an inline test attribute/convention
/// (`#[test]`/`#[cfg(test)]`, JUnit `@Test`, pytest `test_*`). Centralises the
/// "is this a test symbol" decision so orphan/dead-code analysis treats inline
/// tests the same as test-file tests.
pub fn is_test_symbol(node: &SymbolNode) -> bool {
    node.is_test || is_test_path(&node.file)
}

/// Like [`is_test_symbol`] but evaluates the path RELATIVE to `project_root`
/// first. `is_test_path` matches a `tests`/`test` directory component, which on
/// an ABSOLUTE path also matches an ancestor outside the project (e.g. a repo
/// checked out under `…/tests/cache/proj`). Stripping the root confines the
/// check to the project's own layout. Prefer this wherever the root is in hand.
pub fn is_test_symbol_in(node: &SymbolNode, project_root: &Path) -> bool {
    let rel = node.file.strip_prefix(project_root).unwrap_or(&node.file);
    node.is_test || is_test_path(rel)
}

pub fn is_test_path(path: &Path) -> bool {
    // A `tests`/`test` directory component marks test code. Checking COMPONENTS
    // (not a `/tests/` substring) also catches a top-level `tests/` dir, and
    // pairs with `is_test_symbol_in` relativisation to avoid matching a `tests`
    // ancestor that lies OUTSIDE the project root.
    if path
        .components()
        .any(|c| matches!(c.as_os_str().to_str(), Some("tests") | Some("test")))
    {
        return true;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.ends_with("_test.rs") || name.ends_with("_tests.rs") {
        return true;
    }
    if name.ends_with("Test.java") || name.ends_with("Tests.java") {
        return true;
    }
    if name.ends_with(".test.ts")
        || name.ends_with(".test.tsx")
        || name.ends_with(".test.js")
        || name.ends_with(".test.jsx")
        || name.ends_with(".spec.ts")
        || name.ends_with(".spec.tsx")
        || name.ends_with(".spec.js")
        || name.ends_with(".spec.jsx")
    {
        return true;
    }
    if name.starts_with("test_") && (name.ends_with(".py") || name.ends_with(".pyi")) {
        return true;
    }
    if name.ends_with("_test.go") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{EdgeKind, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn insert_node(g: &mut SymbolGraph, name: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            name,
            PathBuf::from("src/lib.rs"),
            0,
            10,
        ))
    }

    fn insert_edge(g: &mut SymbolGraph, from: SymbolId, to: SymbolId) {
        g.insert_edge(DirectEdge::new(
            from,
            to,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("src/lib.rs"),
        ));
    }

    // Impact = the reverse dependents cone: with `a → b` (a calls b), changing
    // `b` breaks `a`, so impact(b) contains a — and impact(a) is empty (what a
    // CALLS does not break when a changes).
    #[test]
    fn direct_impact_one_hop() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a");
        let b = insert_node(&mut g, "b");
        insert_edge(&mut g, a, b);
        let result = compute_impact(&g, b, 5);
        assert_eq!(result.node_count(), 1);
        assert!(result.reachable.iter().any(|n| n.name == "a"));
        // The callee direction is not impact: changing `a` breaks nobody.
        assert_eq!(compute_impact(&g, a, 5).node_count(), 0);
    }

    #[test]
    fn transitive_impact_chain() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a");
        let b = insert_node(&mut g, "b");
        let c = insert_node(&mut g, "c");
        insert_edge(&mut g, a, b);
        insert_edge(&mut g, b, c);
        // a → b → c: changing c breaks b directly and a transitively.
        let result = compute_impact(&g, c, 5);
        assert_eq!(result.node_count(), 2);
    }

    #[test]
    fn depth_limit_respected() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a");
        let b = insert_node(&mut g, "b");
        let c = insert_node(&mut g, "c");
        insert_edge(&mut g, a, b);
        insert_edge(&mut g, b, c);
        // Depth 1 from c reaches only the direct caller b, not a.
        let result = compute_impact(&g, c, 1);
        assert_eq!(result.node_count(), 1);
        assert!(result.reachable.iter().any(|n| n.name == "b"));
        assert!(!result.reachable.iter().any(|n| n.name == "a"));
    }

    #[test]
    fn pick_impact_seed_prefers_called_twin() {
        // Two defs named `as_kebab`: one uncalled, one with a real caller.
        // First-match seeding landed on the uncalled twin and reported zero
        // impact; the picker must seed the definition dependents actually use.
        let mut g = SymbolGraph::new();
        let dead_twin = insert_node(&mut g, "as_kebab");
        let live_twin = insert_node(&mut g, "as_kebab");
        let caller = insert_node(&mut g, "run");
        insert_edge(&mut g, caller, live_twin);
        let picked = pick_impact_seed(&g, "as_kebab").expect("seed found");
        assert_eq!(picked.id, live_twin);
        assert_ne!(picked.id, dead_twin);
        // Ties (no callers anywhere) keep the first candidate, deterministically.
        let mut g2 = SymbolGraph::new();
        let first = insert_node(&mut g2, "dup");
        let _second = insert_node(&mut g2, "dup");
        assert_eq!(pick_impact_seed(&g2, "dup").unwrap().id, first);
    }

    #[test]
    fn cycle_does_not_loop_forever() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a");
        let b = insert_node(&mut g, "b");
        insert_edge(&mut g, a, b);
        insert_edge(&mut g, b, a);
        let result = compute_impact(&g, a, 10);
        assert_eq!(result.node_count(), 1);
    }

    #[test]
    fn isolated_seed_no_impact() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a");
        insert_node(&mut g, "b");
        let result = compute_impact(&g, a, 5);
        assert_eq!(result.node_count(), 0);
        assert_eq!(result.edge_count(), 0);
        assert_eq!(result.depth_reached, 0);
    }

    fn insert_node_at(g: &mut SymbolGraph, name: &str, file: &str, kind: SymbolKind) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            kind,
            name,
            PathBuf::from(file),
            0,
            10,
        ))
    }

    fn insert_call(g: &mut SymbolGraph, from: SymbolId, to: SymbolId, file: &str, conf: f32) {
        g.insert_edge(DirectEdge::new(
            from,
            to,
            EdgeKind::Calls,
            AnalysisOrigin::CompilerDerived,
            Confidence::new(conf),
            PathBuf::from(file),
        ));
    }

    #[test]
    fn is_test_path_recognizes_common_shapes() {
        assert!(is_test_path(&PathBuf::from("src/tests/foo.rs")));
        assert!(is_test_path(&PathBuf::from("crates/foo/tests/a_test.rs")));
        assert!(is_test_path(&PathBuf::from("src/app/FooTest.java")));
        assert!(is_test_path(&PathBuf::from("src/app/foo.test.ts")));
        assert!(is_test_path(&PathBuf::from("tests/test_user.py")));
        assert!(is_test_path(&PathBuf::from("src/pkg/handler_test.go")));
        assert!(
            is_test_path(&PathBuf::from("tests/helpers/util.rs")),
            "top-level tests/ dir"
        );
        assert!(!is_test_path(&PathBuf::from("src/lib.rs")));
        assert!(
            !is_test_path(&PathBuf::from("src/latest/foo.rs")),
            "'latest' is not 'test'"
        );
    }

    #[test]
    fn is_test_symbol_in_strips_root_before_test_check() {
        // The classic absolute-path defect: a project checked out under a path
        // that itself contains a `tests` ancestor (e.g. a vendored fixture under
        // `.../coregraph/tests/cache/proj`) must NOT have all its files flagged
        // as test code. Relativising against the project root confines the check.
        let root = PathBuf::from("/repo/tests/cache/proj");
        let lib = SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "f",
            root.join("src/api.rs"),
            0,
            1,
        );
        assert!(
            !is_test_symbol_in(&lib, &root),
            "src/api.rs under a /tests/ ANCESTOR must not be flagged test"
        );
        // The project's OWN tests/ dir is still detected after relativising.
        let own_test = SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "t",
            root.join("tests/it.rs"),
            0,
            1,
        );
        assert!(
            is_test_symbol_in(&own_test, &root),
            "the project's own tests/ dir is test code"
        );
        // The extractor `is_test` flag is honoured regardless of path.
        let flagged = SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "u",
            root.join("src/x.rs"),
            0,
            1,
        )
        .with_test(true);
        assert!(
            is_test_symbol_in(&flagged, &root),
            "extractor is_test flag wins"
        );
    }

    #[test]
    fn blast_radius_critical_on_large_module_count() {
        let mut g = SymbolGraph::new();
        let seed = insert_node_at(
            &mut g,
            "hot_fn",
            "crates/a/src/lib.rs",
            SymbolKind::Function,
        );
        // Create callers spread across 11 distinct crates — Critical.
        for i in 0..11 {
            let file = format!("crates/crate{}/src/lib.rs", i);
            let caller =
                insert_node_at(&mut g, &format!("caller{}", i), &file, SymbolKind::Function);
            insert_call(&mut g, caller, seed, &file, 0.9);
        }
        let r = compute_risk(&g, seed, 5);
        assert_eq!(r.module_count, 11);
        assert_eq!(r.blast_radius, BlastRadius::Critical);
        assert!(r.risk_score > 0.3);
    }

    #[test]
    fn blast_radius_low_for_small_same_file_fanin() {
        let mut g = SymbolGraph::new();
        let seed = insert_node_at(
            &mut g,
            "cold_fn",
            "crates/a/src/lib.rs",
            SymbolKind::Function,
        );
        let caller = insert_node_at(
            &mut g,
            "caller",
            "crates/a/src/lib.rs",
            SymbolKind::Function,
        );
        insert_call(&mut g, caller, seed, "crates/a/src/lib.rs", 0.85);
        let r = compute_risk(&g, seed, 5);
        assert_eq!(r.blast_radius, BlastRadius::Low);
        assert_eq!(r.caller_count, 1);
    }

    #[test]
    fn affected_tests_populates_from_test_paths() {
        let mut g = SymbolGraph::new();
        let seed = insert_node_at(
            &mut g,
            "hot_fn",
            "crates/a/src/lib.rs",
            SymbolKind::Function,
        );
        let test = insert_node_at(
            &mut g,
            "hot_fn_test",
            "crates/a/tests/a_test.rs",
            SymbolKind::Function,
        );
        insert_call(&mut g, test, seed, "crates/a/tests/a_test.rs", 0.95);
        let r = compute_risk(&g, seed, 5);
        assert_eq!(r.affected_tests.len(), 1);
        assert_eq!(r.affected_tests[0].name, "hot_fn_test");
    }

    #[test]
    fn affected_tests_excludes_non_executable_nodes() {
        // Synthetic / non-executable nodes that happen to sit in a test path —
        // string-literal refs (env_ref::/api_path::), config keys, enum variants
        // — must NOT be reported as "affected tests". A test is an executable
        // unit (Function/Method); listing an env_ref:: marker under "Affected
        // tests" is meaningless output.
        let mut g = SymbolGraph::new();
        let seed = insert_node_at(
            &mut g,
            "hot_fn",
            "crates/a/src/lib.rs",
            SymbolKind::Function,
        );
        let real_test = insert_node_at(
            &mut g,
            "hot_fn_test",
            "crates/a/tests/a_test.rs",
            SymbolKind::Function,
        );
        let env_ref = insert_node_at(
            &mut g,
            "env_ref::API_KEY",
            "crates/a/tests/a_test.rs",
            SymbolKind::StringLiteral,
        );
        let variant = insert_node_at(
            &mut g,
            "WsSubtype.SERVER",
            "crates/a/tests/a_test.rs",
            SymbolKind::EnumVariant,
        );
        insert_call(&mut g, real_test, seed, "crates/a/tests/a_test.rs", 0.95);
        insert_call(&mut g, env_ref, seed, "crates/a/tests/a_test.rs", 0.95);
        insert_call(&mut g, variant, seed, "crates/a/tests/a_test.rs", 0.95);
        let r = compute_risk(&g, seed, 5);
        let names: Vec<_> = r.affected_tests.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"hot_fn_test"),
            "real test must be listed: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.starts_with("env_ref::")),
            "synthetic string-literal ref must not be a test: {names:?}"
        );
        assert!(
            !names.contains(&"WsSubtype.SERVER"),
            "enum variant must not be a test: {names:?}"
        );
    }

    #[test]
    fn path_confidence_multiplies_edge_confidences() {
        let e1 = DirectEdge::new(
            SymbolId(1),
            SymbolId(2),
            EdgeKind::Calls,
            AnalysisOrigin::CompilerDerived,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        );
        let e2 = DirectEdge::new(
            SymbolId(2),
            SymbolId(3),
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        );
        // current_confidence uses base_score, not stored confidence.
        // 0.99 * 0.85 = 0.8415
        let p = path_confidence(&[e1, e2]);
        assert!((p - 0.8415).abs() < 1e-3, "got {}", p);
    }

    #[test]
    fn risk_level_thresholds() {
        assert_eq!(RiskLevel::from_score(0.1), RiskLevel::Low);
        assert_eq!(RiskLevel::from_score(0.5), RiskLevel::Medium);
        assert_eq!(RiskLevel::from_score(0.7), RiskLevel::High);
        assert_eq!(RiskLevel::from_score(0.9), RiskLevel::Critical);
    }
}

#[cfg(test)]
mod symbols_enclosing_line_tests {
    use super::*;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use coregraph_graph::SymbolGraph;
    use std::io::Write;

    /// Write `content` to a unique temp file and return its path.
    fn write_tmp(content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cg-enclosing-{}-{}.rs",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos(),
        ));
        std::fs::File::create(&path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
        path
    }

    #[test]
    fn symbols_enclosing_line_finds_symbol_at_its_declaration_line() {
        // Source layout (bytes):
        //   offset  0..10  : "// line 0\n"     (line 0)
        //   offset 10..25  : "fn target() {}\n" (line 1, 15 bytes)
        //   offset 25..39  : "fn other() {}\n"  (line 2, 14 bytes)
        let src = "// line 0\nfn target() {}\nfn other() {}\n";
        let file = write_tmp(src);

        let mut g = SymbolGraph::new();
        // "target" spans bytes 10..25 (the full "fn target() {}" token range).
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "target",
            file.clone(),
            10,
            25,
        ));
        // "other" spans bytes 25..39.
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "other",
            file.clone(),
            25,
            39,
        ));

        // Line 1 (byte offset 10) is inside "target" (10 <= 10 < 25).
        let hits = symbols_enclosing_line(&g, &file, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "target");

        // Line 2 (byte offset 25) is inside "other" (25 <= 25 < 39).
        let hits2 = symbols_enclosing_line(&g, &file, 2);
        assert_eq!(hits2.len(), 1);
        assert_eq!(hits2[0].name, "other");

        // Line 0 (byte offset 0) — no symbol starts before byte 10.
        let hits0 = symbols_enclosing_line(&g, &file, 0);
        assert!(hits0.is_empty());

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn symbols_enclosing_line_returns_empty_for_missing_file() {
        let g = SymbolGraph::new();
        let hits = symbols_enclosing_line(&g, std::path::Path::new("/nonexistent/file.rs"), 5);
        assert!(hits.is_empty());
    }
}
