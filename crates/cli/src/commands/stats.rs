use crate::global_opts::{GlobalOpts, OutputFormat as GlobalFormat};
use clap::Args;
use coregraph_core::{EdgeKind, SymbolId, SymbolKind};
use coregraph_graph::SymbolGraph;
use std::collections::HashMap;

#[derive(Args)]
pub struct StatsArgs {
    /// Include detailed breakdown: symbol/edge kind histograms, per-crate
    /// counts, top in-degree symbols, heaviest files.
    #[arg(long, default_value_t = false)]
    pub breakdown: bool,

    /// Top-N cut-off for breakdown lists (default 20)
    #[arg(long, default_value_t = 20)]
    pub top: usize,
}

pub fn run(args: StatsArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    // Thin-client path: if the daemon is running for this project, delegate.
    // We still fall through to in-process for --breakdown since the daemon
    // wire format doesn't carry the full graph.
    if !args.breakdown && crate::ipc::ensure_running(globals) {
        let req = crate::ipc::Request {
            method: "stats".to_string(),
            params: serde_json::json!({}),
            project: globals.project_root(),
        };
        if let Ok(resp) = crate::ipc::send(&req) {
            if resp.ok {
                println!("{}", resp.body);
                return Ok(());
            }
        }
    }

    let (graph, file_count) = crate::graph_loader::load_project_graph(&globals.project)?;
    let symbols = graph.node_count();
    let edges = graph.edge_count();
    match globals.output_format {
        GlobalFormat::Json => {
            println!(
                "{{\"files\":{},\"symbols\":{},\"edges\":{}}}",
                file_count, symbols, edges
            );
        }
        _ => {
            println!("Indexed {} files", file_count);
            println!("symbols: {}", symbols);
            println!("edges:   {}", edges);
        }
    }

    if args.breakdown {
        print_breakdown(&graph, args.top);
    }
    Ok(())
}

fn print_breakdown(graph: &SymbolGraph, top: usize) {
    println!("\n## Symbol kinds");
    let mut by_kind: HashMap<SymbolKind, u32> = HashMap::new();
    for n in graph.nodes() {
        *by_kind.entry(n.kind.clone()).or_insert(0) += 1;
    }
    let mut kinds: Vec<_> = by_kind.iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in &kinds {
        println!("  {:16} {}", format!("{:?}", k), n);
    }

    println!("\n## Edge kinds");
    let mut by_edge: HashMap<EdgeKind, u32> = HashMap::new();
    let mut by_origin: HashMap<String, u32> = HashMap::new();
    let mut by_trust_model: HashMap<String, u32> = HashMap::new();
    for e in graph.edges() {
        *by_edge.entry(e.kind.clone()).or_insert(0) += 1;
        *by_origin.entry(format!("{:?}", e.origin)).or_insert(0) += 1;
        *by_trust_model
            .entry(format!("{:?}", e.trust_model()))
            .or_insert(0) += 1;
    }
    let mut ev: Vec<_> = by_edge.iter().collect();
    ev.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in &ev {
        println!("  {:16} {}", format!("{:?}", k), n);
    }
    println!("\n## Analysis origins");
    let mut tv: Vec<_> = by_origin.iter().collect();
    tv.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in &tv {
        println!("  {:20} {}", k, n);
    }
    println!("\n## Trust models");
    let mut tv: Vec<_> = by_trust_model.iter().collect();
    tv.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in &tv {
        println!("  {:16} {}", k, n);
    }

    println!("\n## Per-crate symbol/edge counts");
    let mut per_crate: HashMap<String, (u32, u32)> = HashMap::new();
    for n in graph.nodes() {
        let c = crate_of(n.file.to_str().unwrap_or(""));
        per_crate.entry(c).or_insert((0, 0)).0 += 1;
    }
    for e in graph.edges() {
        let c = crate_of(e.evidence_file.to_str().unwrap_or(""));
        per_crate.entry(c).or_insert((0, 0)).1 += 1;
    }
    let mut crates: Vec<_> = per_crate.iter().collect();
    crates.sort_by_key(|x| std::cmp::Reverse(x.1 .0));
    println!("  {:24} {:>8} {:>8}", "crate", "symbols", "edges");
    for (c, (n, e)) in &crates {
        println!("  {:24} {:>8} {:>8}", c, n, e);
    }

    println!("\n## Top {} most-referenced symbols (in-degree)", top);
    let mut in_deg: HashMap<SymbolId, u32> = HashMap::new();
    for e in graph.edges() {
        *in_deg.entry(e.to).or_insert(0) += 1;
    }
    let mut ranked: Vec<_> = in_deg.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));
    for (id, count) in ranked.iter().take(top) {
        if let Some(n) = graph.get_node(**id) {
            println!(
                "  {:>5}  {:22} [{:?}] @ {}",
                count,
                truncate(&n.name, 22),
                n.kind,
                n.file.display()
            );
        }
    }

    println!("\n## Top {} files by symbol count", top);
    let mut per_file: HashMap<std::path::PathBuf, u32> = HashMap::new();
    for n in graph.nodes() {
        *per_file.entry(n.file.to_path_buf()).or_insert(0) += 1;
    }
    let mut files: Vec<_> = per_file.iter().collect();
    files.sort_by(|a, b| b.1.cmp(a.1));
    for (f, c) in files.iter().take(top) {
        println!("  {:>5}  {}", c, f.display());
    }
}

fn crate_of(path: &str) -> String {
    let segs: Vec<_> = path.split('/').collect();
    for (i, s) in segs.iter().enumerate() {
        if *s == "crates" && i + 1 < segs.len() {
            return segs[i + 1].to_string();
        }
    }
    if path.contains("tests/fixtures") {
        "tests/fixtures".to_string()
    } else {
        "other".to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_of_extracts_coregraph_crate_name() {
        assert_eq!(crate_of("./crates/graph/src/lib.rs"), "graph");
        assert_eq!(crate_of("/abs/crates/cli/src/main.rs"), "cli");
    }

    #[test]
    fn crate_of_groups_test_fixtures() {
        assert_eq!(
            crate_of("./tests/fixtures/java-spring/UserService.java"),
            "tests/fixtures"
        );
    }

    #[test]
    fn crate_of_other_for_unmatched() {
        assert_eq!(crate_of("./Cargo.toml"), "other");
    }

    #[test]
    fn truncate_under_limit_preserved() {
        assert_eq!(truncate("foo", 10), "foo");
    }

    #[test]
    fn truncate_over_limit_adds_ellipsis() {
        let r = truncate("abcdefghijklmnop", 5);
        assert!(r.ends_with('…'));
        assert_eq!(r.chars().count(), 5);
    }
}
