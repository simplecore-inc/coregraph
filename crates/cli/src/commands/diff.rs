//! `coregraph diff <git-ref>` — analyse the impact of a git diff.
//!
//! Workflow: ask git for the files that differ between the working
//! tree (or `--to`) and the user-supplied base ref, then walk the
//! symbol graph to surface every symbol whose evidence touches one of
//! those files plus everything those symbols can reach. The result is
//! "what does this branch break" without needing the user to know
//! every public API name.
//!
//! The computation and rendering live in `crate::dispatch`
//! (`compute_diff_summary` / `render_diff_summary`) so the in-process path here
//! and the daemon `diff_summary` handler produce byte-identical output.

use clap::Args;

use crate::global_opts::{GlobalOpts, OutputFormat};

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
    let project_root = globals.project_root();
    let depth = args.max_depth.unwrap_or(globals.hop_limit);

    // Thin-client path: the daemon's `cached_diff_summary` runs the same
    // `compute_diff_summary` against its cached graph and shares
    // `render_diff_summary`, so a daemon-served result is byte-identical.
    // Forward base/to/depth/exclude. Falls through to in-process below when the
    // daemon is absent or the call fails.
    let params = serde_json::json!({
        "base": args.base,
        "to": args.to,
        "max_depth": depth,
        "exclude_tests": args.exclude_tests,
        "output_format": match globals.output_format {
            OutputFormat::Json => "json",
            OutputFormat::Llm => "llm",
            OutputFormat::Human => "human",
        },
    });
    if let Some(body) = crate::ipc::try_daemon(globals, "diff_summary", params) {
        println!("{}", body);
        return Ok(());
    }

    // Build the project graph via the canonical loader shared with every other
    // command, then run the shared computation + renderer.
    let graph = crate::graph_loader::load_project_graph_only(&project_root)?;
    let summary = crate::dispatch::compute_diff_summary(
        &graph,
        &project_root,
        &args.base,
        &args.to,
        depth,
        args.exclude_tests,
    )?;
    println!(
        "{}",
        crate::dispatch::render_diff_summary(&summary, globals.output_format)
    );
    Ok(())
}
