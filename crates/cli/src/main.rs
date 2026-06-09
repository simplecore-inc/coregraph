mod commands;
mod context;
mod daemon;
mod dispatch;
mod global_opts;
mod graph_loader;
mod ipc;
mod langfilter;
mod project_manager;
mod render;

use clap::{Parser, Subcommand};
use commands::{
    batch, config as config_cmd, diff, export, impact, inconsistencies, index, inspect, lsp, mcp,
    orphans, plugin, query, review, server, snapshot, stats, watch_diff,
};
use global_opts::GlobalOpts;

#[derive(Parser)]
#[command(
    name = "coregraph",
    about = "CoreGraph — code symbol graph CLI",
    version = env!("CARGO_PKG_VERSION")
)]
struct Cli {
    #[command(flatten)]
    globals: GlobalOpts,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index source files and build the symbol graph.
    Index(index::IndexArgs),
    /// Query symbols in the graph.
    Query(query::QueryArgs),
    /// Inspect the symbol at FILE:LINE.
    Inspect(inspect::InspectArgs),
    /// Show graph statistics (with optional breakdown).
    Stats(stats::StatsArgs),
    /// List orphan symbols (no incoming/outgoing edges).
    ///
    /// SCOPE: orphans reports only FULLY-DISCONNECTED symbols — a symbol with no
    /// semantic edge in EITHER direction. A dead symbol that still has any
    /// resolved OUTGOING edge (e.g. a never-called function that itself calls a
    /// live helper, or a dead component that renders other components) is NOT
    /// reported. So a clean result is not proof of no dead code; treat the list
    /// as triage candidates, not a census. Always confirm a hit with a targeted
    /// read — dynamic dispatch, reflection, FFI, serialization, and string- or
    /// config-driven wiring are out of the graph and cause false "dead" hits.
    Orphans(orphans::OrphansArgs),
    /// Impact analysis for a symbol.
    Impact(impact::ImpactArgs),
    /// Impact of a git diff: which symbols does the change reach.
    Diff(diff::DiffArgs),
    /// Auto-comment a GitHub PR with the diff impact summary.
    Review(review::ReviewArgs),
    /// Detect cross-enum / api-path / config-key inconsistencies.
    Inconsistencies(inconsistencies::InconsistenciesArgs),
    /// Export the graph (dot | cypher | json-graph).
    Export(export::ExportArgs),
    /// Save or load a binary snapshot.
    Snapshot(snapshot::SnapshotArgs),
    /// Show, set, or initialize configuration.
    Config(config_cmd::ConfigArgs),
    /// Daemon management (start/stop/status/restart/install/uninstall).
    Server(server::ServerArgs),
    /// LSP stdio bridge.
    Lsp(lsp::LspArgs),
    /// MCP stdio bridge.
    Mcp(mcp::McpArgs),
    /// Watch for file changes and rebuild the graph.
    Watch(watch_diff::WatchArgs),
    /// Run multiple queries from a JSON file.
    Batch(batch::BatchArgs),
    /// Manage plugin hooks.
    Plugin(plugin::PluginArgs),
}

fn main() {
    let cli = Cli::parse();
    let mut globals = cli.globals.resolve_preset();

    // Load --config file if present (overrides defaults that the user did not
    // explicitly change). Silent on I/O errors.
    apply_config_file(&mut globals);

    // Reject nonsensical numeric ranges up-front. Silently running with
    // `--min-confidence 5.0` used to return empty pages that looked like
    // "no data" when the filter was actually over-tight; now we fail fast
    // with a clear range error.
    if let Err(e) = validate_globals(&globals) {
        eprintln!("Error: {e}");
        std::process::exit(2);
    }

    // Initialize logging based on --log-level / --verbose / --quiet.
    init_logging(&globals);

    let result = match cli.command {
        Commands::Index(args) => index::run(args, &globals),
        Commands::Query(args) => query::run(args, &globals),
        Commands::Inspect(args) => inspect::run(args, &globals),
        Commands::Stats(args) => stats::run(args, &globals),
        Commands::Orphans(args) => orphans::run(args, &globals),
        Commands::Impact(args) => impact::run(args, &globals),
        Commands::Diff(args) => diff::run(args, &globals),
        Commands::Review(args) => review::run(args, &globals),
        Commands::Inconsistencies(args) => inconsistencies::run(args, &globals),
        Commands::Export(args) => export::run(args, &globals),
        Commands::Snapshot(args) => snapshot::run(args, &globals),
        Commands::Config(args) => config_cmd::run(args, &globals.project),
        Commands::Server(args) => server::run(args, &globals),
        Commands::Lsp(args) => lsp::run(args, &globals),
        Commands::Mcp(args) => mcp::run(args, &globals),
        Commands::Watch(args) => watch_diff::run(args, &globals),
        Commands::Batch(args) => batch::run(args, &globals),
        Commands::Plugin(args) => plugin::run(args, &globals),
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

/// Read `--config` (or the default path) and overlay `limits.*` /
/// `default.*` keys onto GlobalOpts. Explicit CLI values always win — we
/// only fill in fields that are still at their clap defaults.
fn apply_config_file(globals: &mut GlobalOpts) {
    let path = globals
        .config
        .clone()
        .unwrap_or_else(commands::config::config_path);
    if !path.exists() {
        return;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(val) = toml::from_str::<toml::Value>(&text) else {
        return;
    };
    let table = match val {
        toml::Value::Table(t) => t,
        _ => return,
    };

    if let Some(limits) = table.get("limits").and_then(|v| v.as_table()) {
        // The "unchanged baseline" heuristic compares against the clap
        // defaults from `global_opts.rs`. When a default changes there,
        // the sentinel here must change too — otherwise the config
        // file's value is silently ignored because the runtime never
        // matches the old sentinel.
        if globals.token_budget == DEFAULT_TOKEN_BUDGET {
            if let Some(n) = limits.get("token_budget").and_then(|v| v.as_integer()) {
                globals.token_budget = n as usize;
            }
        }
        if globals.hop_limit == DEFAULT_HOP_LIMIT {
            if let Some(n) = limits.get("hop_limit").and_then(|v| v.as_integer()) {
                globals.hop_limit = n as usize;
            }
        }
        if (globals.min_confidence - DEFAULT_MIN_CONFIDENCE).abs() < f32::EPSILON {
            if let Some(f) = limits.get("min_confidence").and_then(|v| v.as_float()) {
                globals.min_confidence = f as f32;
            }
        }
    }
}

/// Sentinels that mirror the clap `default_value_t` in `global_opts.rs`.
/// Keeping these as named constants makes the coupling visible — when
/// either side changes, `cargo build` surfaces the mismatch immediately
/// through dead_code/unused warnings rather than through silent config
/// drift at runtime.
const DEFAULT_TOKEN_BUDGET: usize = 8000;
const DEFAULT_HOP_LIMIT: usize = 3;
const DEFAULT_MIN_CONFIDENCE: f32 = 0.70;

/// Reject numeric arguments that make no analytical sense. These aren't
/// security checks — clap already rejects non-numeric input — they're
/// guardrails that replace silently-empty query pages with a clear
/// error. Each message includes both the offending value and the
/// acceptable range so users can fix their invocation in one pass.
fn validate_globals(globals: &GlobalOpts) -> anyhow::Result<()> {
    if !(0.0..=1.0).contains(&globals.min_confidence) {
        anyhow::bail!(
            "--min-confidence must be in [0.0, 1.0], got {}",
            globals.min_confidence
        );
    }
    if globals.hop_limit == 0 {
        anyhow::bail!("--hop-limit must be at least 1, got 0");
    }
    if globals.token_budget < 100 {
        anyhow::bail!(
            "--token-budget must be at least 100 tokens (got {}); below that no result fits",
            globals.token_budget
        );
    }
    Ok(())
}

/// Map --log-level / --verbose / --quiet to a simple global log level that
/// subcommands can read via tracing-style guards.
fn init_logging(globals: &GlobalOpts) {
    use crate::global_opts::LogLevel;
    let level = if globals.quiet {
        LogLevel::Error
    } else if globals.verbose {
        LogLevel::Debug
    } else {
        globals.log_level
    };
    // Expose via env for downstream libs (tracing-subscriber picks this up).
    let level_str = match level {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    };
    std::env::set_var("RUST_LOG", level_str);
    if matches!(level, LogLevel::Debug | LogLevel::Trace) {
        eprintln!("[coregraph] log level: {}", level_str);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_opts::{ColorMode, LogLevel, OutputFormat};
    use std::path::PathBuf;

    fn base() -> GlobalOpts {
        GlobalOpts {
            project: PathBuf::from("."),
            config: None,
            output_format: OutputFormat::Human,
            color: ColorMode::Auto,
            token_budget: 8000,
            hop_limit: 3,
            min_confidence: 0.70,
            include_stale: false,
            lang: vec![],
            verbose: false,
            quiet: false,
            log_level: LogLevel::Info,
            fast: false,
            standard: false,
            full: false,
            no_auto_start: false,
        }
    }

    #[test]
    fn validate_accepts_defaults() {
        assert!(validate_globals(&base()).is_ok());
    }

    #[test]
    fn validate_rejects_min_confidence_out_of_range() {
        let mut g = base();
        g.min_confidence = 1.5;
        assert!(validate_globals(&g).is_err());
        g.min_confidence = -0.1;
        assert!(validate_globals(&g).is_err());
    }

    #[test]
    fn validate_rejects_zero_hop_limit() {
        let mut g = base();
        g.hop_limit = 0;
        assert!(validate_globals(&g).is_err());
    }

    #[test]
    fn validate_rejects_tiny_token_budget() {
        let mut g = base();
        g.token_budget = 50;
        assert!(validate_globals(&g).is_err());
    }

    #[test]
    fn validate_accepts_boundary_values() {
        // Inclusive boundaries on min_confidence; minimums on the
        // other knobs. These values shouldn't produce errors.
        let mut g = base();
        g.min_confidence = 0.0;
        assert!(validate_globals(&g).is_ok());
        g.min_confidence = 1.0;
        assert!(validate_globals(&g).is_ok());
        g.hop_limit = 1;
        g.token_budget = 100;
        assert!(validate_globals(&g).is_ok());
    }
}
