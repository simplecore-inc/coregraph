//! `coregraph diff <git-ref>` — analyse the impact of a git diff.
//!
//! Workflow: ask git for the files that differ between the working
//! tree (or `--to`) and the user-supplied base ref, then walk the
//! symbol graph to surface every symbol whose evidence touches one of
//! those files plus everything those symbols can reach. The result is
//! "what does this branch break" without needing the user to know
//! every public API name.

use clap::Args;
use coregraph_core::SymbolId;
use coregraph_query::{compute_impact, is_test_path};
use coregraph_watcher::GitDiffStrategy;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::global_opts::{GlobalOpts, OutputFormat};
use crate::graph_loader::load_project_graph_only;

#[derive(Args)]
pub struct DiffArgs {
    /// Base git ref to diff against (e.g. `main`, `HEAD~3`, a commit
    /// SHA, or a tag).
    pub base: String,

    /// Compare this ref instead of the working tree. Defaults to
    /// `HEAD` (so working-tree edits are included).
    #[arg(long, default_value = "HEAD")]
    pub to: String,

    /// Maximum impact propagation depth from each touched symbol.
    /// Mirrors the global `--hop-limit`; when omitted the global
    /// default applies.
    #[arg(long)]
    pub max_depth: Option<usize>,

    /// Skip symbols defined under test directories.
    #[arg(long, default_value_t = false)]
    pub exclude_tests: bool,
}

pub fn run(args: DiffArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let project_root =
        std::fs::canonicalize(&globals.project).unwrap_or_else(|_| globals.project.clone());

    // 1. Ask git for the changed files.
    let changed = GitDiffStrategy::changed_files_between(&project_root, &args.base, &args.to)?;
    if changed.is_empty() {
        match globals.output_format {
            OutputFormat::Json => {
                println!(
                    "{}",
                    serde_json::json!({
                        "base": args.base,
                        "to": args.to,
                        "changed_files": 0,
                        "touched_symbols": 0,
                        "reachable_symbols": 0,
                    })
                );
            }
            _ => {
                println!(
                    "No files changed between {} and {} — nothing to analyse.",
                    args.base, args.to
                );
            }
        }
        return Ok(());
    }

    // 2. Build the project graph via the canonical loader shared with
    // every other command, so the impact reach stays consistent.
    let graph = load_project_graph_only(&project_root)?;
    let canonical_changed: HashSet<PathBuf> = changed
        .iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect();

    // 3. Find every symbol whose definition file matches the diff.
    let mut touched: Vec<&coregraph_core::SymbolNode> = graph
        .nodes()
        .filter(|n| {
            let nf = std::fs::canonicalize(&n.file).unwrap_or_else(|_| n.file.to_path_buf());
            canonical_changed.contains(&nf)
        })
        .collect();
    if args.exclude_tests {
        touched.retain(|n| !is_test_path(&n.file));
    }
    // Sort for stable output independent of HashMap iteration order.
    touched.sort_by(|a, b| (a.file.as_ref(), a.span_start).cmp(&(b.file.as_ref(), b.span_start)));

    // 4. Compute the union of impact cones from each touched symbol.
    let depth = args.max_depth.unwrap_or(globals.hop_limit);
    let mut reached: HashSet<SymbolId> = HashSet::new();
    for seed in &touched {
        let result = compute_impact(&graph, seed.id, depth);
        for n in result.reachable {
            reached.insert(n.id);
        }
    }
    // Don't double-count the seeds themselves.
    for seed in &touched {
        reached.remove(&seed.id);
    }

    // 5. Render.
    match globals.output_format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "base": args.base,
                    "to": args.to,
                    "changed_files": changed.len(),
                    "touched_symbols": touched.len(),
                    "reachable_symbols": reached.len(),
                    "max_depth": depth,
                    "touched": touched.iter().map(|n| serde_json::json!({
                        "name": n.name,
                        "kind": format!("{:?}", n.kind),
                        "file": n.file.display().to_string(),
                    })).collect::<Vec<_>>(),
                })
            );
        }
        OutputFormat::Llm => {
            println!("## Diff impact: {}..{}", args.base, args.to);
            println!("- changed_files: {}", changed.len());
            println!("- touched_symbols: {}", touched.len());
            println!("- reachable_symbols: {} (depth {})", reached.len(), depth);
            for n in touched.iter().take(50) {
                println!("  - {} [{:?}] @ {}", n.name, n.kind, n.file.display());
            }
        }
        OutputFormat::Human => {
            println!(
                "Diff {}..{}: {} file(s), {} touched symbol(s), {} reachable (depth {})",
                args.base,
                args.to,
                changed.len(),
                touched.len(),
                reached.len(),
                depth,
            );
            for n in touched.iter().take(20) {
                println!("  • {} [{:?}] @ {}", n.name, n.kind, n.file.display());
            }
            if touched.len() > 20 {
                println!("  … and {} more", touched.len() - 20);
            }
        }
    }
    Ok(())
}
