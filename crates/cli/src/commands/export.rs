use crate::global_opts::GlobalOpts;
use clap::{Args, ValueEnum};
use coregraph_core::SymbolId;
use coregraph_extractor::build_graph;
use coregraph_graph::SymbolGraph;
use std::collections::HashSet;

#[derive(Args)]
pub struct ExportArgs {
    /// Output format.
    #[arg(long, default_value = "dot")]
    pub format: ExportFormat,

    /// Restrict export to a subgraph centered on this symbol (+hop-limit hops).
    #[arg(long)]
    pub subgraph: Option<String>,
}

#[derive(Copy, Clone, ValueEnum)]
pub enum ExportFormat {
    Dot,
    Cypher,
    JsonGraph,
}

pub fn run(args: ExportArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let (graph, _) = build_graph(&globals.project)?;
    let mut keep: Option<std::collections::HashSet<SymbolId>> = args
        .subgraph
        .as_deref()
        .map(|name| subgraph_ids(&graph, name, globals.hop_limit));

    // Further restrict to --lang filter if given.
    if !globals.lang.is_empty() {
        let lang_filter: std::collections::HashSet<SymbolId> = graph
            .nodes()
            .filter(|n| crate::langfilter::match_langs(&globals.lang, &n.file))
            .map(|n| n.id)
            .collect();
        keep = Some(match keep {
            Some(s) => s.intersection(&lang_filter).copied().collect(),
            None => lang_filter,
        });
    }

    let min_conf = globals.min_confidence;
    let output = match args.format {
        ExportFormat::Dot => emit_dot(&graph, keep.as_ref(), min_conf),
        ExportFormat::Cypher => emit_cypher(&graph, keep.as_ref(), min_conf),
        ExportFormat::JsonGraph => emit_json_graph(&graph, keep.as_ref(), min_conf),
    };
    println!("{}", output);
    Ok(())
}

fn edge_passes_confidence(edge: &coregraph_core::DirectEdge, min_conf: f32) -> bool {
    edge.confidence.0 >= min_conf
}

/// Return the set of SymbolIds reachable within `hop_limit` hops of any node
/// whose name matches `name` (substring).
fn subgraph_ids(graph: &SymbolGraph, name: &str, hop_limit: usize) -> HashSet<SymbolId> {
    let seeds: Vec<SymbolId> = graph
        .nodes()
        .filter(|n| n.name.contains(name))
        .map(|n| n.id)
        .collect();

    let mut keep: HashSet<SymbolId> = seeds.iter().copied().collect();
    let mut frontier: HashSet<SymbolId> = keep.clone();
    for _ in 0..hop_limit {
        let mut next: HashSet<SymbolId> = HashSet::new();
        for e in graph.edges() {
            if frontier.contains(&e.from) && !keep.contains(&e.to) {
                next.insert(e.to);
            }
            if frontier.contains(&e.to) && !keep.contains(&e.from) {
                next.insert(e.from);
            }
        }
        if next.is_empty() {
            break;
        }
        keep.extend(next.iter().copied());
        frontier = next;
    }
    keep
}

fn included(n: &coregraph_core::SymbolNode, keep: Option<&HashSet<SymbolId>>) -> bool {
    keep.is_none_or(|k| k.contains(&n.id))
}

fn emit_dot(graph: &SymbolGraph, keep: Option<&HashSet<SymbolId>>, min_conf: f32) -> String {
    let mut out = String::from("digraph coregraph {\n  rankdir=LR;\n  node [shape=box];\n");
    for n in graph.nodes().filter(|n| included(n, keep)) {
        out.push_str(&format!(
            "  n{} [label=\"{}\\n({:?})\"];\n",
            n.id.0,
            dot_escape(&n.name),
            n.kind
        ));
    }
    for e in graph.edges() {
        if !edge_passes_confidence(e, min_conf) {
            continue;
        }
        if let (Some(f), Some(t)) = (graph.get_node(e.from), graph.get_node(e.to)) {
            if !included(f, keep) || !included(t, keep) {
                continue;
            }
            out.push_str(&format!(
                "  n{} -> n{} [label=\"{:?}\"];\n",
                e.from.0, e.to.0, e.kind
            ));
        }
    }
    out.push_str("}\n");
    out
}

fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn emit_cypher(graph: &SymbolGraph, keep: Option<&HashSet<SymbolId>>, min_conf: f32) -> String {
    let mut out = String::new();
    for n in graph.nodes().filter(|n| included(n, keep)) {
        out.push_str(&format!(
            "CREATE (n{}:{:?} {{id: {}, name: {}, file: {}}});\n",
            n.id.0,
            n.kind,
            n.id.0,
            cypher_string(&n.name),
            cypher_string(&n.file.to_string_lossy()),
        ));
    }
    for e in graph.edges() {
        if !edge_passes_confidence(e, min_conf) {
            continue;
        }
        if let (Some(f), Some(t)) = (graph.get_node(e.from), graph.get_node(e.to)) {
            if !included(f, keep) || !included(t, keep) {
                continue;
            }
            out.push_str(&format!(
                "MATCH (a {{id: {}}}), (b {{id: {}}}) CREATE (a)-[:{:?} {{confidence: {:.3}}}]->(b);\n",
                e.from.0,
                e.to.0,
                e.kind,
                e.confidence.0
            ));
        }
    }
    out
}

fn cypher_string(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn emit_json_graph(graph: &SymbolGraph, keep: Option<&HashSet<SymbolId>>, min_conf: f32) -> String {
    // CLI export ships the full edge vocabulary; only the daemon/atlas path prunes kinds.
    json_graph_string(graph, keep, min_conf, &HashSet::new(), true)
}

/// Serialize the graph (optionally restricted to `keep`) as a json-graph
/// document. Shared by the CLI `export --format json-graph` path (pretty)
/// and the daemon's `export_graph` method (compact, served from memory
/// for the atlas viewer).
///
/// `exclude_kinds` drops edges whose `{:?}` kind name is listed — the atlas
/// uses it to omit name-resolution plumbing (e.g. `Resolves`, the bulk of
/// edges on a large graph) that the viewer never renders, so it never reaches
/// the browser. An empty set keeps every edge kind.
pub fn json_graph_string(
    graph: &SymbolGraph,
    keep: Option<&HashSet<SymbolId>>,
    min_conf: f32,
    exclude_kinds: &HashSet<String>,
    pretty: bool,
) -> String {
    let nodes: Vec<_> = graph
        .nodes()
        .filter(|n| included(n, keep))
        .map(|n| {
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "kind": format!("{:?}", n.kind),
                "file": n.file.display().to_string(),
                "span_start": n.span_start,
                "span_end": n.span_end,
            })
        })
        .collect();
    let edges: Vec<_> = graph
        .edges()
        .filter_map(|e| {
            if !edge_passes_confidence(e, min_conf) {
                return None;
            }
            let kind = format!("{:?}", e.kind);
            if exclude_kinds.contains(&kind) {
                return None;
            }
            let f = graph.get_node(e.from)?;
            let t = graph.get_node(e.to)?;
            if !included(f, keep) || !included(t, keep) {
                return None;
            }
            Some(serde_json::json!({
                "from": e.from,
                "to": e.to,
                "kind": kind,
                "trust": format!("{:?}", e.origin),
                "origin": format!("{:?}", e.origin),
                "trust_model": format!("{:?}", e.trust_model()),
                "confidence": e.confidence.0,
                "stale_evidence_count": e.stale_evidence_count,
                "current_confidence": e.current_confidence(),
            }))
        })
        .collect();
    // Full graph size, independent of the `keep` / `min_conf` / `exclude_kinds`
    // filtering applied to the emitted `nodes`/`edges` arrays. The atlas shows
    // this as the project's edge total so the loaded view agrees with the
    // project picker (which reports the daemon's full edge_count) even when the
    // payload omits edge kinds such as `Resolves`.
    let root = serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "total_nodes": graph.node_count(),
        "total_edges": graph.edge_count(),
    });
    if pretty {
        serde_json::to_string_pretty(&root).unwrap_or_default()
    } else {
        serde_json::to_string(&root).unwrap_or_default()
    }
}
