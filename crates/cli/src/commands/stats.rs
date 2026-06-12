use crate::global_opts::{GlobalOpts, OutputFormat as GlobalFormat};
use clap::Args;
use coregraph_core::{EdgeKind, SymbolId, SymbolKind};
use coregraph_graph::SymbolGraph;
use coregraph_manifest::parse_project;
use coregraph_query::{is_impact_bearing, is_impact_node};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct StatsArgs {
    /// Include detailed breakdown: symbol/edge kind histograms, per-package
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
    if !args.breakdown {
        // Forward the output format so the daemon renders json/llm/human like
        // the in-process path; previously it sent empty params and the daemon
        // always replied with human text, ignoring `--output-format json`.
        let params = serde_json::json!({
            "output_format": match globals.output_format {
                GlobalFormat::Json => "json",
                GlobalFormat::Llm => "llm",
                GlobalFormat::Human => "human",
            },
        });
        if let Some(body) = crate::ipc::try_daemon(globals, "stats", params) {
            println!("{}", body);
            return Ok(());
        }
    }

    // Canonical root so in-process node paths match the daemon's.
    let (graph, file_count) = crate::graph_loader::load_project_graph(&globals.project_root())?;
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
        print_breakdown(&graph, args.top, &globals.project_root());
    }
    Ok(())
}

/// Maps source files to the manifest package that owns them. Built once per
/// stats invocation from coregraph-manifest (Cargo workspaces, npm/pnpm
/// workspaces, Maven/Gradle modules, …) — no path conventions are assumed.
struct PackageMap {
    /// (package dir relative to root, package name); member packages sorted
    /// deepest-first so nested members win, the root package (".") last.
    entries: Vec<(PathBuf, String)>,
    root: PathBuf,
}

impl PackageMap {
    fn load(root: &Path) -> Self {
        let entries = parse_project(root)
            .map(|m| {
                let mut v: Vec<(PathBuf, String)> = m
                    .packages
                    .into_iter()
                    .filter(|p| !p.path.as_os_str().is_empty())
                    .map(|p| (p.path, p.name))
                    .collect();
                Self::sort_entries(&mut v);
                v
            })
            // parse_project always returns Ok today (individual parser failures are
            // logged to stderr inside parse_project); unwrap_or_default is a safety
            // net in case that contract ever changes.
            .unwrap_or_default();
        Self {
            root: root.to_path_buf(),
            entries,
        }
    }

    /// Deepest path first; the root package (".") sorts last so it acts as the
    /// catch-all only after every member package had its chance.
    fn sort_entries(entries: &mut [(PathBuf, String)]) {
        entries.sort_by_key(|(p, _)| {
            std::cmp::Reverse(if p == Path::new(".") {
                0
            } else {
                p.components().count()
            })
        });
    }

    fn package_of(&self, file: &Path) -> String {
        if file.as_os_str().is_empty() {
            // Invariant: an empty file path only occurs on synthetic
            // ExternalPackage placeholder nodes (see the extractor's
            // external-node minting) — real source symbols always carry a
            // file anchor. Bucket them as "(external)" here; the per-file
            // section labels the same set "(external packages — no file)".
            return "(external)".to_string();
        }
        let rel = file.strip_prefix(&self.root).unwrap_or(file);
        for (dir, name) in &self.entries {
            if dir == Path::new(".") || rel.starts_with(dir) {
                return name.clone();
            }
        }
        "(no package)".to_string()
    }
}

fn print_breakdown(graph: &SymbolGraph, top: usize, root: &Path) {
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

    println!("\n## Per-package symbol/edge counts");
    let packages = PackageMap::load(root);
    let mut per_pkg: HashMap<String, (u32, u32)> = HashMap::new();
    for n in graph.nodes() {
        let p = packages.package_of(n.file.as_ref());
        per_pkg.entry(p).or_insert((0, 0)).0 += 1;
    }
    for e in graph.edges() {
        let p = packages.package_of(e.evidence_file.as_ref());
        per_pkg.entry(p).or_insert((0, 0)).1 += 1;
    }
    let mut pkgs: Vec<_> = per_pkg.iter().collect();
    pkgs.sort_by_key(|x| std::cmp::Reverse(x.1 .0));
    println!("  {:32} {:>8} {:>8}", "package", "symbols", "edges");
    for (c, (n, e)) in &pkgs {
        println!("  {:32} {:>8} {:>8}", truncate(c, 32), n, e);
    }

    println!(
        "\n## Top {} most-referenced symbols (in-degree; containers excluded)",
        top
    );
    // Mirrors impact's `incoming_impact_degree`: impact-bearing edge kinds
    // only, and both endpoints must be dependent-eligible symbols.
    let in_deg = impact_in_degree(graph);
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
    let mut per_file: HashMap<PathBuf, u32> = HashMap::new();
    let mut external_symbols: u32 = 0;
    for n in graph.nodes() {
        // ExternalPackage placeholders have no file anchor; a nameless row in
        // a "top files" table is a display bug, so count them separately.
        if n.file.as_os_str().is_empty() {
            external_symbols += 1;
            continue;
        }
        *per_file.entry(n.file.to_path_buf()).or_insert(0) += 1;
    }
    let mut files: Vec<_> = per_file.iter().collect();
    files.sort_by(|a, b| b.1.cmp(a.1));
    for (f, c) in files.iter().take(top) {
        println!("  {:>5}  {}", c, f.display());
    }
    if external_symbols > 0 {
        println!("  {:>5}  (external packages — no file)", external_symbols);
    }
}

/// In-degree restricted the same way impact counts dependents
/// (`incoming_impact_degree`): impact-bearing edge kinds only, and both
/// endpoints must be dependent-eligible symbols (no File / doc containers).
/// Without the edge/source filters every function gains in-degree from its
/// own file's Contains edge and from file-level fallback Resolves edges.
fn impact_in_degree(graph: &SymbolGraph) -> HashMap<SymbolId, u32> {
    let mut in_deg: HashMap<SymbolId, u32> = HashMap::new();
    for e in graph.edges() {
        if !is_impact_bearing(&e.kind) {
            continue;
        }
        let src_ok = graph
            .get_node(e.from)
            .is_some_and(|n| is_impact_node(&n.kind));
        let tgt_ok = graph
            .get_node(e.to)
            .is_some_and(|n| is_impact_node(&n.kind));
        if !src_ok || !tgt_ok {
            continue;
        }
        *in_deg.entry(e.to).or_insert(0) += 1;
    }
    in_deg
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

    fn map_with(entries: Vec<(&str, &str)>, root: &str) -> PackageMap {
        let mut v: Vec<(PathBuf, String)> = entries
            .into_iter()
            .map(|(p, n)| (PathBuf::from(p), n.to_string()))
            .collect();
        PackageMap::sort_entries(&mut v);
        PackageMap {
            entries: v,
            root: PathBuf::from(root),
        }
    }

    #[test]
    fn package_of_maps_workspace_members_by_path_prefix() {
        let m = map_with(
            vec![
                ("packages/ui", "@x/ui"),
                ("packages/ui-icons", "@x/ui-icons"),
            ],
            "/repo",
        );
        assert_eq!(
            m.package_of(Path::new("/repo/packages/ui/src/a.ts")),
            "@x/ui"
        );
        assert_eq!(
            m.package_of(Path::new("/repo/packages/ui-icons/b.ts")),
            "@x/ui-icons",
            "longest/exact component match must win, not raw string prefix"
        );
    }

    #[test]
    fn package_of_falls_back_for_unmatched_and_external() {
        let m = map_with(vec![("crates/core", "coregraph-core")], "/repo");
        assert_eq!(m.package_of(Path::new("/repo/README.md")), "(no package)");
        assert_eq!(m.package_of(Path::new("")), "(external)");
    }

    #[test]
    fn package_of_root_package_is_catch_all_but_sorted_last() {
        let m = PackageMap {
            entries: {
                let mut v = vec![
                    (PathBuf::from("packages/ui"), "@x/ui".to_string()),
                    (PathBuf::from("."), "root-pkg".to_string()),
                ];
                PackageMap::sort_entries(&mut v);
                v
            },
            root: PathBuf::from("/repo"),
        };
        assert_eq!(
            m.package_of(Path::new("/repo/packages/ui/src/a.ts")),
            "@x/ui"
        );
        assert_eq!(m.package_of(Path::new("/repo/src/main.ts")), "root-pkg");
    }

    #[test]
    fn package_map_loads_npm_workspace() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let root = dir.path();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"mono","private":true,"workspaces":["packages/*"]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("packages/ui")).unwrap();
        std::fs::write(
            root.join("packages/ui/package.json"),
            r#"{"name":"@x/ui","version":"1.0.0","exports":{".":"./src/index.ts"}}"#,
        )
        .unwrap();
        let m = PackageMap::load(root);
        assert_eq!(
            m.package_of(&root.join("packages/ui/src/index.ts")),
            "@x/ui"
        );
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

    #[test]
    fn impact_in_degree_counts_only_impact_bearing_symbol_edges() {
        use coregraph_core::edge::{AnalysisOrigin, Confidence};
        use coregraph_core::{DirectEdge, SymbolNode};
        let mut g = SymbolGraph::new();
        let file = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::File,
            "a.ts",
            "a.ts",
            0,
            100,
        ));
        let f = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "f",
            "a.ts",
            0,
            10,
        ));
        let caller = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "caller",
            "b.ts",
            0,
            10,
        ));
        let mk = |from, to, kind| {
            DirectEdge::new(
                from,
                to,
                kind,
                AnalysisOrigin::SyntaxMatched,
                Confidence::new(0.85),
                PathBuf::from("b.ts"),
            )
        };
        g.insert_edge(mk(caller, f, EdgeKind::Calls)); // counts
        g.insert_edge(mk(file, f, EdgeKind::Contains)); // structural — must not count
        g.insert_edge(mk(file, f, EdgeKind::Resolves)); // file-level fallback source — must not count
        let deg = impact_in_degree(&g);
        assert_eq!(deg.get(&f).copied(), Some(1), "only the Calls edge counts");
        assert!(!deg.contains_key(&file), "container nodes never appear");
    }
}
