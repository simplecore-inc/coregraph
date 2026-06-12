use crate::global_opts::{GlobalOpts, OutputFormat};
use clap::{Args, ValueEnum};
use coregraph_extractor::{build_graph, find_doc_param_drift};
use coregraph_query::{
    find_inconsistencies_with, InconsistencyCategory, InconsistencyOptions, PathExcluder,
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

impl Category {
    /// Wire form forwarded to the daemon (`cached_inconsistencies`) and the
    /// kebab label used everywhere else.
    fn as_kebab(self) -> &'static str {
        match self {
            Category::EnumMismatch => "enum-mismatch",
            Category::ApiPath => "api-path",
            Category::ConfigKey => "config-key",
            Category::DocDrift => "doc-drift",
        }
    }

    /// Map the CLI category to the query-layer category. Doc-drift has no
    /// query-layer analog — it is handled on a separate path.
    fn to_query_category(self) -> Option<InconsistencyCategory> {
        match self {
            Category::EnumMismatch => Some(InconsistencyCategory::EnumMismatch),
            Category::ApiPath => Some(InconsistencyCategory::ApiPath),
            Category::ConfigKey => Some(InconsistencyCategory::ConfigKey),
            Category::DocDrift => None,
        }
    }
}

pub fn run(args: InconsistenciesArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    // Thin-client path: the daemon's `cached_inconsistencies` mirrors this
    // function exactly (shared renderers, same category routing + lang/exclude
    // filtering), so a daemon-served result is byte-identical. Falls through to
    // the in-process path below when the daemon is absent or the call fails.
    let params = serde_json::json!({
        "category": args.category.map(Category::as_kebab),
        "lang": globals.lang,
        "output_format": match globals.output_format {
            OutputFormat::Json => "json",
            OutputFormat::Llm => "llm",
            OutputFormat::Human => "human",
        },
    });
    if let Some(body) = crate::ipc::try_daemon(globals, "inconsistencies", params) {
        println!("{}", body);
        return Ok(());
    }

    // Canonical root so in-process node paths and exclude classification match
    // the daemon's (which canonicalizes its routing key).
    let root = globals.project_root();
    let (graph, _) = build_graph(&root)?;
    let excluder = PathExcluder::from_project_root(&root);

    // Documentation drift has a different report shape (single node), so it is
    // reported through its own opt-in category path.
    if matches!(args.category, Some(Category::DocDrift)) {
        let reports: Vec<_> = find_doc_param_drift(&graph)
            .into_iter()
            .filter(|r| crate::langfilter::match_langs(&globals.lang, &r.file))
            .filter(|r| !excluder.is_excluded(&r.file))
            .collect();
        println!(
            "{}",
            crate::dispatch::render_doc_drift(&reports, globals.output_format)
        );
        return Ok(());
    }

    // Single set + filter, identical to the daemon's `cached_inconsistencies`,
    // so the local and daemon report sets (and their order) match exactly.
    let category = args.category.and_then(Category::to_query_category);
    // Config-driven category disabling applies to the run-everything mode;
    // an explicit --category is a stronger signal and always runs.
    // Intentional design: all enabled detectors run and results are
    // post-filtered by category; the retain only re-enables an explicitly
    // requested category (detectors are cheap relative to graph build).
    let mut opts = InconsistencyOptions::from_project_root(&root);
    if let Some(c) = category {
        opts.disabled.retain(|d| *d != c);
    }
    let reports: Vec<_> = find_inconsistencies_with(&graph, &opts)
        .into_iter()
        .filter(|r| category.is_none_or(|c| r.category == c))
        .filter(|r| {
            crate::langfilter::match_langs(&globals.lang, &r.node_a.file)
                || crate::langfilter::match_langs(&globals.lang, &r.node_b.file)
        })
        .filter(|r| !excluder.is_excluded(&r.node_a.file) && !excluder.is_excluded(&r.node_b.file))
        .collect();

    // Single shared renderer (also used by the daemon path) so the output is
    // identical whether the query ran here in-process or through the daemon.
    println!(
        "{}",
        crate::dispatch::render_inconsistencies(&reports, globals.output_format)
    );
    Ok(())
}
