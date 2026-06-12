use crate::global_opts::{GlobalOpts, OutputFormat};
use clap::Args;
use coregraph_extractor::build_graph;
use coregraph_graph::{save_snapshot, GraphEpoch};
use coregraph_query::noise_candidates;
use std::path::PathBuf;

#[derive(Args)]
pub struct IndexArgs {
    /// Accepted for compatibility; currently a no-op. The CLI `index` always
    /// rebuilds the whole graph from source (it never reuses a snapshot), so
    /// there is no incremental default for this to override. Full-vs-fast
    /// reindex modes apply only to the daemon's `reindex` IPC method.
    #[arg(long, default_value_t = false)]
    pub full: bool,

    /// Detect changes only; don't rebuild the graph.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Emit indexing statistics (file count, symbol count, elapsed).
    #[arg(long, default_value_t = false)]
    pub stats: bool,

    /// Save the resulting graph to this snapshot path.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
}

pub fn run(args: IndexArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let root = &globals.project;
    // Materialise a default `.coregraph/config.toml` on first run so the
    // per-project knob surface is discoverable without making the user
    // remember `coregraph config init --local`. Existing files are
    // never overwritten.
    crate::commands::config::ensure_local_default(root);
    for w in crate::commands::config::validate_project_config(root) {
        eprintln!("[coregraph] WARNING: {w}");
    }
    let started = std::time::Instant::now();

    if args.dry_run {
        let count = coregraph_watcher::GitDiffStrategy::changed_files_since_head(root)
            .unwrap_or_default()
            .len();
        match globals.output_format {
            OutputFormat::Json => println!(
                "{}",
                serde_json::json!({"changed_files": count, "dry_run": true})
            ),
            _ => println!("dry-run: {} file(s) changed since HEAD", count),
        }
        return Ok(());
    }

    // tree-sitter + stack-graphs pass. `build_graph` runs every extractor
    // to produce symbol nodes, then `coregraph-stack`'s StackGraphsBackend
    // stitches cross-file name resolutions (Java/TS/JS/Python) with a
    // syntactic fallback for the rest. This is the whole pipeline — every
    // layer is incremental and requires no external toolchain.
    let (graph, files) = build_graph(root)?;

    let elapsed = started.elapsed();

    if let Some(out) = &args.snapshot {
        save_snapshot(
            out,
            &graph,
            GraphEpoch::zero().next(),
            std::time::SystemTime::now(),
        )?;
    }

    let print_stats = args.stats || globals.verbose;
    let body = match globals.output_format {
        OutputFormat::Json => serde_json::json!({
            "files": files,
            "symbols": graph.node_count(),
            "edges": graph.edge_count(),
            "elapsed_ms": elapsed.as_millis(),
            "full": args.full,
        })
        .to_string(),
        OutputFormat::Llm => format!(
            "## Index result\n- files: {}\n- symbols: {}\n- edges: {}\n- elapsed_ms: {}\n",
            files,
            graph.node_count(),
            graph.edge_count(),
            elapsed.as_millis()
        ),
        OutputFormat::Human => {
            if print_stats {
                format!(
                    "Index complete — {} files, {} symbols, {} edges ({}ms)",
                    files,
                    graph.node_count(),
                    graph.edge_count(),
                    elapsed.as_millis()
                )
            } else {
                format!(
                    "Index complete — {} files processed, {} symbols, {} edges",
                    files,
                    graph.node_count(),
                    graph.edge_count(),
                )
            }
        }
    };
    println!("{}", body);
    if matches!(globals.output_format, OutputFormat::Human) {
        let noisy = noise_candidates(&graph);
        if !noisy.is_empty() {
            println!(
                "note: {} file(s) contribute an outsized share of data symbols (config keys / string literals / doc sections) — if they are generated or data files,\n      add them to [index].exclude in .coregraph/config.toml:",
                noisy.len()
            );
            for f in &noisy {
                println!(
                    "  {:>6} symbols ({:>3}%)  {}",
                    f.data_symbols,
                    f.share_pct,
                    f.file.display()
                );
            }
        }
    }
    Ok(())
}
