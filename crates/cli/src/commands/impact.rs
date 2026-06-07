use crate::global_opts::{GlobalOpts, OutputFormat};
use clap::Args;
use coregraph_query::{compute_impact, compute_risk, ImpactRisk};

#[derive(Args)]
pub struct ImpactArgs {
    /// Symbol name to analyze impact for.
    pub symbol: String,

    /// Compute the transitive closure (requires --max-depth).
    #[arg(long, default_value_t = false)]
    pub transitive: bool,

    /// Maximum depth of impact propagation.
    #[arg(long, default_value_t = 5)]
    pub max_depth: usize,

    /// Include confidence-weighted risk scoring (blast radius, affected tests,
    /// 4-factor risk score per docs/graph-model.md §6.7).
    #[arg(long, default_value_t = false)]
    pub risk: bool,
}

pub fn run(args: ImpactArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    // An empty seed triggers the name-contains fallback below, which
    // matches every symbol and returns a useless "impact of whole repo"
    // reachable set. Fail fast instead.
    if args.symbol.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "symbol name must not be empty (got {:?})",
            args.symbol
        ));
    }

    let graph = crate::graph_loader::load_project_graph_only(&globals.project)?;

    let depth = if args.transitive {
        args.max_depth
    } else {
        globals.hop_limit
    };

    let seed = graph
        .nodes()
        .find(|n| n.name == args.symbol)
        .or_else(|| graph.nodes().find(|n| n.name.contains(&args.symbol)))
        .cloned();
    let seed = match seed {
        Some(s) => s,
        None => {
            println!("Symbol '{}' not found in graph.", args.symbol);
            return Ok(());
        }
    };

    let excluder = coregraph_query::PathExcluder::from_project_root(&globals.project);

    let mut result = compute_impact(&graph, seed.id, depth);
    result.reachable.retain(|n| {
        crate::langfilter::match_langs(&globals.lang, &n.file) && !excluder.is_excluded(&n.file)
    });

    let risk = if args.risk {
        let mut r = compute_risk(&graph, seed.id, depth);
        r.affected_tests
            .retain(|t| !excluder.is_excluded(std::path::Path::new(&t.file)));
        Some(r)
    } else {
        None
    };

    match globals.output_format {
        OutputFormat::Json => {
            let mut json = serde_json::json!({
                "symbol": args.symbol,
                "reachable": result.reachable.len(),
                "edges": result.edges.len(),
                "depth": result.depth_reached,
                "transitive": args.transitive,
                "nodes": result.reachable.iter().map(|n| serde_json::json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": format!("{:?}", n.kind),
                    "file": n.file.display().to_string(),
                })).collect::<Vec<_>>(),
            });
            if let Some(r) = &risk {
                json["risk"] = risk_as_json(r);
            }
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        OutputFormat::Llm => {
            println!("## Impact: {}", args.symbol);
            println!("- reachable: {}", result.reachable.len());
            println!("- edges: {}", result.edges.len());
            println!("- depth_reached: {}", result.depth_reached);
            println!("- transitive: {}", args.transitive);
            if let Some(r) = &risk {
                println!(
                    "- risk_score: {:.2} ({})",
                    r.risk_score,
                    r.risk_level_label()
                );
                println!(
                    "- blast_radius: {} ({} modules, {} callers)",
                    r.blast_radius_label(),
                    r.module_count,
                    r.caller_count
                );
                println!(
                    "- confidence_weighted_impact: {:.3}",
                    r.confidence_weighted_impact
                );
                println!("- affected_tests: {}", r.affected_tests.len());
            }
            println!();
            println!("| name | kind | file |\n|---|---|---|");
            for n in &result.reachable {
                println!("| {} | {:?} | {} |", n.name, n.kind, n.file.display());
            }
            if let Some(r) = &risk {
                if !r.affected_tests.is_empty() {
                    println!("\n### Affected tests");
                    println!("| test | distance | path_confidence |\n|---|---|---|");
                    for t in &r.affected_tests {
                        println!("| {} | {} | {:.2} |", t.name, t.distance, t.path_confidence);
                    }
                }
            }
        }
        OutputFormat::Human => {
            println!(
                "Impact of '{}': {} reachable symbols, {} edges, depth {}{}",
                args.symbol,
                result.reachable.len(),
                result.edges.len(),
                result.depth_reached,
                if args.transitive { " (transitive)" } else { "" }
            );
            if let Some(r) = &risk {
                println!(
                    "  Risk Score: {:.2} ({})",
                    r.risk_score,
                    r.risk_level_label()
                );
                println!(
                    "  Blast Radius: {} ({} modules, {} callers)",
                    r.blast_radius_label(),
                    r.module_count,
                    r.caller_count
                );
                println!(
                    "  Confidence-Weighted Impact: {:.3}",
                    r.confidence_weighted_impact
                );
                println!("  Affected tests: {}", r.affected_tests.len());
                for t in r.affected_tests.iter().take(10) {
                    println!(
                        "    {} (distance {}, path_confidence {:.2}) — {}",
                        t.name, t.distance, t.path_confidence, t.file
                    );
                }
            }
            for n in &result.reachable {
                println!("  {} [{:?}] — {}", n.name, n.kind, n.file.display());
            }
        }
    }
    Ok(())
}

fn risk_as_json(r: &ImpactRisk) -> serde_json::Value {
    serde_json::json!({
        "score": r.risk_score,
        "level": r.risk_level_label(),
        "blast_radius": r.blast_radius_label(),
        "module_count": r.module_count,
        "caller_count": r.caller_count,
        "visibility_score": r.visibility_score,
        "caller_factor": r.caller_factor,
        "module_factor": r.module_factor,
        "impact_kind_factor": r.impact_kind_factor,
        "confidence_weighted_impact": r.confidence_weighted_impact,
        "affected_tests": r.affected_tests,
    })
}
