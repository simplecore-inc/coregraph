use crate::global_opts::{GlobalOpts, OutputFormat};
use clap::{Args, ValueEnum};
use coregraph_extractor::{build_graph, find_doc_param_drift};
use coregraph_query::{
    find_api_path_mismatches, find_config_key_mismatches, find_enum_mismatches,
    InconsistencyCategory,
};

#[derive(Args)]
pub struct InconsistenciesArgs {
    /// Restrict to a specific inconsistency category.
    #[arg(long)]
    pub category: Option<Category>,
}

#[derive(Copy, Clone, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum Category {
    EnumMismatch,
    ApiPath,
    ConfigKey,
    /// Documentation drift: a `@param` / `:param` naming a parameter the
    /// function signature no longer has (JS/TS/Java/Python).
    DocDrift,
}

pub fn run(args: InconsistenciesArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let (graph, _) = build_graph(&globals.project)?;

    // Documentation drift has a different report shape (no second node), so it
    // is reported through its own opt-in category path.
    if matches!(args.category, Some(Category::DocDrift)) {
        return run_doc_drift(&graph, globals);
    }

    let mut reports = Vec::new();
    match args.category {
        None => {
            reports.extend(find_enum_mismatches(&graph));
            reports.extend(find_api_path_mismatches(&graph));
            reports.extend(find_config_key_mismatches(&graph));
        }
        Some(Category::EnumMismatch) => reports.extend(find_enum_mismatches(&graph)),
        Some(Category::ApiPath) => reports.extend(find_api_path_mismatches(&graph)),
        Some(Category::ConfigKey) => reports.extend(find_config_key_mismatches(&graph)),
        Some(Category::DocDrift) => unreachable!("handled above"),
    }

    let excluder = coregraph_query::PathExcluder::from_project_root(&globals.project);
    let reports: Vec<_> = reports
        .into_iter()
        .filter(|r| {
            crate::langfilter::match_langs(&globals.lang, &r.node_a.file)
                || crate::langfilter::match_langs(&globals.lang, &r.node_b.file)
        })
        .filter(|r| !excluder.is_excluded(&r.node_a.file) && !excluder.is_excluded(&r.node_b.file))
        .collect();

    match globals.output_format {
        OutputFormat::Json => {
            let json: Vec<_> = reports
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "category": r.category.label(),
                        "shared_value": r.shared_value,
                        "a": {
                            "name": strip_marker(&r.node_a.name),
                            "file": r.node_a.file.display().to_string(),
                        },
                        "b": {
                            "name": strip_marker(&r.node_b.name),
                            "file": r.node_b.file.display().to_string(),
                        },
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        OutputFormat::Llm => {
            println!("## Inconsistencies");
            println!("- count: {}", reports.len());
            if !reports.is_empty() {
                println!("\n| category | detail | a | a_file | b | b_file |");
                println!("|---|---|---|---|---|---|");
                for r in &reports {
                    println!(
                        "| {} | {} | {} | {} | {} | {} |",
                        r.category.label(),
                        r.shared_value,
                        strip_marker(&r.node_a.name),
                        r.node_a.file.display(),
                        strip_marker(&r.node_b.name),
                        r.node_b.file.display()
                    );
                }
            }
        }
        OutputFormat::Human => {
            if reports.is_empty() {
                println!("No inconsistencies detected.");
            } else {
                println!("Inconsistencies ({}):", reports.len());
                for r in &reports {
                    match r.category {
                        InconsistencyCategory::EnumMismatch => {
                            println!("  [enum-mismatch] '{}' appears in:", r.shared_value);
                            println!(
                                "    - {} ({})",
                                strip_marker(&r.node_a.name),
                                r.node_a.file.display()
                            );
                            println!(
                                "    - {} ({})",
                                strip_marker(&r.node_b.name),
                                r.node_b.file.display()
                            );
                        }
                        InconsistencyCategory::ApiPath => {
                            println!("  [api-path] {}", r.shared_value);
                            println!("    - {}", r.node_a.file.display());
                            println!("    - {}", r.node_b.file.display());
                        }
                        InconsistencyCategory::ConfigKey => {
                            println!("  [config-key] {}", r.shared_value);
                            println!(
                                "    - {} ({})",
                                strip_marker(&r.node_a.name),
                                r.node_a.file.display()
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Render documentation-drift findings (`--category doc-drift`). A separate path
/// because `DocDriftReport` has no second node — it is one symbol whose doc and
/// signature disagree.
fn run_doc_drift(graph: &coregraph_graph::SymbolGraph, globals: &GlobalOpts) -> anyhow::Result<()> {
    let excluder = coregraph_query::PathExcluder::from_project_root(&globals.project);
    let reports: Vec<_> = find_doc_param_drift(graph)
        .into_iter()
        .filter(|r| crate::langfilter::match_langs(&globals.lang, &r.file))
        .filter(|r| !excluder.is_excluded(&r.file))
        .collect();

    match globals.output_format {
        OutputFormat::Json => {
            let json: Vec<_> = reports
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "category": "doc-drift",
                        "symbol": r.symbol,
                        "file": r.file.display().to_string(),
                        "detail": r.detail,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        OutputFormat::Llm => {
            println!("## Documentation Drift");
            println!("- count: {}", reports.len());
            if !reports.is_empty() {
                println!("\n| symbol | file | detail |");
                println!("|---|---|---|");
                for r in &reports {
                    println!("| {} | {} | {} |", r.symbol, r.file.display(), r.detail);
                }
            }
        }
        OutputFormat::Human => {
            if reports.is_empty() {
                println!("No documentation drift detected.");
            } else {
                println!("Documentation drift ({}):", reports.len());
                for r in &reports {
                    println!("  [doc-drift] {} ({})", r.symbol, r.file.display());
                    println!("    - {}", r.detail);
                }
            }
        }
    }
    Ok(())
}

/// Removes internal marker prefixes from display names so end-users don't
/// see `api_path::/foo` or `config_ref::foo.bar` in the output.
fn strip_marker(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("api_path::") {
        rest.to_string()
    } else if let Some(rest) = name.strip_prefix("config_ref::") {
        rest.to_string()
    } else {
        name.to_string()
    }
}
