use crate::global_opts::{GlobalOpts, OutputFormat};
use clap::Args;
use coregraph_query::{compute_impact, compute_risk, PathExcluder};
use std::path::Path;

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

    let depth = if args.transitive {
        args.max_depth
    } else {
        globals.hop_limit
    };

    // Thin-client path: the daemon's `cached_impact` mirrors this function
    // exactly (shared `render_impact`, same seed match, same lang/exclude
    // filtering), so a daemon-served result is byte-identical. Forward the
    // resolved depth so the daemon does not re-derive it. Falls through to the
    // in-process path below when the daemon is absent or the call fails.
    let params = serde_json::json!({
        "symbol": args.symbol,
        "depth": depth,
        "transitive": args.transitive,
        "risk": args.risk,
        "lang": globals.lang,
        "output_format": match globals.output_format {
            OutputFormat::Json => "json",
            OutputFormat::Llm => "llm",
            OutputFormat::Human => "human",
        },
    });
    if let Some(body) = crate::ipc::try_daemon(globals, "impact", params) {
        println!("{}", body);
        return Ok(());
    }

    let graph = crate::graph_loader::load_project_graph_only(&globals.project)?;

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

    let excluder = PathExcluder::from_project_root(&globals.project);

    let mut result = compute_impact(&graph, seed.id, depth);
    result.reachable.retain(|n| {
        crate::langfilter::match_langs(&globals.lang, &n.file) && !excluder.is_excluded(&n.file)
    });

    let risk = if args.risk {
        let mut r = compute_risk(&graph, seed.id, depth);
        r.affected_tests
            .retain(|t| !excluder.is_excluded(Path::new(&t.file)));
        Some(r)
    } else {
        None
    };

    // Single shared renderer (also used by the daemon path, `cached_impact`) so
    // `--output-format {human,llm,json}` is identical whether the query ran here
    // in-process or through the daemon.
    println!(
        "{}",
        crate::dispatch::render_impact(
            &args.symbol,
            &result,
            risk.as_ref(),
            args.transitive,
            globals.output_format
        )
    );
    Ok(())
}
