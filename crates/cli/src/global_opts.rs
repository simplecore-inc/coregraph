//! Global CLI options shared across every subcommand.
//! These are defined at the top-level `Cli` struct and propagated via `flatten`.

use clap::{Args, ValueEnum};
use std::path::PathBuf;

/// Global options supported by every subcommand.
#[derive(Args, Debug, Clone)]
pub struct GlobalOpts {
    /// Project root directory.
    #[arg(short = 'C', long, global = true, default_value = ".")]
    pub project: PathBuf,

    /// Configuration file path (default: $XDG_CONFIG_HOME/coregraph/config.toml).
    #[arg(short = 'c', long, global = true)]
    pub config: Option<PathBuf>,

    /// Output format.
    #[arg(long, global = true, default_value = "human")]
    pub output_format: OutputFormat,

    /// Color output mode.
    #[arg(long, global = true, default_value = "auto")]
    pub color: ColorMode,

    /// Maximum tokens for LLM output.
    #[arg(long, global = true, default_value_t = 8000)]
    pub token_budget: usize,

    /// Maximum graph traversal depth.
    #[arg(long, global = true, default_value_t = 3)]
    pub hop_limit: usize,

    /// Minimum edge confidence filter (0.0–1.0). The default 0.70 admits
    /// every non-pattern-matched edge: `SyntaxMatched` imports (baseline
    /// 0.7225) and above all survive, while `PatternMatched` guesses
    /// (baseline 0.60) are filtered out. Raising to 0.85 filters all
    /// `SyntaxMatched` signal; 0.90 keeps only `NameResolved` and
    /// `CompilerDerived`. Pass 0.0 for the full unfiltered graph.
    #[arg(long, global = true, default_value_t = 0.70)]
    pub min_confidence: f32,

    /// Include stale nodes/edges in results.
    #[arg(long, global = true, default_value_t = false)]
    pub include_stale: bool,

    /// Filter by language (repeatable): java, rust, typescript, ...
    #[arg(long, global = true)]
    pub lang: Vec<String>,

    /// Verbose logging.
    #[arg(short = 'v', long, global = true, default_value_t = false)]
    pub verbose: bool,

    /// Quiet (errors only).
    #[arg(short = 'q', long, global = true, default_value_t = false)]
    pub quiet: bool,

    /// Log level.
    #[arg(long, global = true, default_value = "info")]
    pub log_level: LogLevel,

    /// Fast analysis preset: `--min-confidence 0.7 --hop-limit 1 --token-budget 2000`.
    #[arg(long, global = true, default_value_t = false)]
    pub fast: bool,

    /// Standard analysis preset (default values).
    #[arg(long, global = true, default_value_t = false)]
    pub standard: bool,

    /// Full analysis preset: `--hop-limit 5 --include-stale --token-budget 16000`.
    #[arg(long, global = true, default_value_t = false)]
    pub full: bool,

    /// Don't auto-spawn the daemon when the IPC socket is missing.
    /// Each command falls back to an in-process `build_graph` instead.
    /// Equivalent to setting `COREGRAPH_NO_AUTO_START=1` in the
    /// environment.
    #[arg(long, global = true, default_value_t = false)]
    pub no_auto_start: bool,
}

impl GlobalOpts {
    /// Apply `--fast` / `--full` preset overrides to the resolved options.
    /// `--standard` is the default and has no effect beyond explicitness.
    pub fn resolve_preset(mut self) -> Self {
        if self.fast {
            // `fast` tightens both hop-limit and confidence: it only
            // follows `NameResolved` / `CompilerDerived` evidence.
            if (self.min_confidence - 0.70).abs() < f32::EPSILON
                || (self.min_confidence - 0.85).abs() < f32::EPSILON
                || self.min_confidence == 0.0
            {
                self.min_confidence = 0.9;
            }
            if self.hop_limit == 3 {
                self.hop_limit = 1;
            }
            if self.token_budget == 8000 {
                self.token_budget = 2000;
            }
        } else if self.full {
            // `full` loosens the filters to expose even low-confidence
            // pattern matches — most useful for manual exploration.
            if (self.min_confidence - 0.70).abs() < f32::EPSILON
                || (self.min_confidence - 0.85).abs() < f32::EPSILON
            {
                self.min_confidence = 0.0;
            }
            if self.hop_limit == 3 {
                self.hop_limit = 5;
            }
            self.include_stale = true;
            if self.token_budget == 8000 {
                self.token_budget = 16000;
            }
        }
        self
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Human,
    Llm,
    Json,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> GlobalOpts {
        GlobalOpts {
            project: PathBuf::from("."),
            config: None,
            output_format: OutputFormat::Human,
            color: ColorMode::Auto,
            token_budget: 8000,
            hop_limit: 3,
            // Mirror the clap default so preset-trigger logic sees the
            // same "unchanged baseline" it sees on the real CLI path.
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
    fn fast_preset_applies_values() {
        let opts = GlobalOpts {
            fast: true,
            ..base()
        }
        .resolve_preset();
        // `fast` tightens confidence above the new baseline so only
        // NameResolved+ edges pass.
        assert!((opts.min_confidence - 0.9).abs() < 1e-6);
        assert_eq!(opts.hop_limit, 1);
        assert_eq!(opts.token_budget, 2000);
    }

    #[test]
    fn full_preset_drops_confidence_to_show_everything() {
        // `full` explicitly loosens the filter so manual exploration
        // can see pattern-matched and low-confidence edges.
        let opts = GlobalOpts {
            full: true,
            ..base()
        }
        .resolve_preset();
        assert!(opts.min_confidence.abs() < 1e-6);
    }

    #[test]
    fn full_preset_applies_values() {
        let opts = GlobalOpts {
            full: true,
            ..base()
        }
        .resolve_preset();
        assert_eq!(opts.hop_limit, 5);
        assert!(opts.include_stale);
        assert_eq!(opts.token_budget, 16000);
    }

    #[test]
    fn standard_preset_is_noop() {
        let opts = GlobalOpts {
            standard: true,
            ..base()
        }
        .resolve_preset();
        assert_eq!(opts.hop_limit, 3);
        assert_eq!(opts.token_budget, 8000);
    }

    #[test]
    fn explicit_values_preserved_over_preset() {
        // User passed --fast AND --hop-limit 10 — explicit wins.
        let opts = GlobalOpts {
            fast: true,
            hop_limit: 10,
            ..base()
        }
        .resolve_preset();
        assert_eq!(opts.hop_limit, 10);
    }
}
