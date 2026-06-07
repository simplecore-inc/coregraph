use clap::{Args, Subcommand};
use coregraph_graph::{HookEntry, HookKind, HookRegistry};

#[derive(Args)]
pub struct PluginArgs {
    #[command(subcommand)]
    pub command: PluginCommands,
}

#[derive(Subcommand)]
pub enum PluginCommands {
    /// List all registered plugin hooks
    List,
    /// Run a dry-run of the default registry against a directory,
    /// firing all pre/post hooks so you can verify they execute.
    Run {
        /// Directory to index (default: current directory)
        #[arg(default_value = ".")]
        path: String,
    },
}

/// Build the default hook registry with **executable** built-in hooks.
/// These demonstrate the callback API and also provide useful feedback
/// when invoked via `build_graph_with_hooks`.
pub fn default_registry() -> HookRegistry {
    let mut reg = HookRegistry::new();

    // Pre-analysis: verify the root exists and is readable.
    reg.register(HookEntry::pre(
        "builtin-pre-scan",
        "Verifies the source directory exists and is a directory",
        |root| {
            if !root.exists() {
                return Err(anyhow::anyhow!(
                    "source directory not found: {}",
                    root.display()
                ));
            }
            if !root.is_dir() {
                return Err(anyhow::anyhow!(
                    "source path is not a directory: {}",
                    root.display()
                ));
            }
            eprintln!("[hook:builtin-pre-scan] OK — {}", root.display());
            Ok(())
        },
    ));

    // Post-analysis: print a concise summary to stderr.
    reg.register(HookEntry::post(
        "builtin-post-scan",
        "Emits a summary line (node/edge counts) to stderr after extraction",
        |graph| {
            eprintln!(
                "[hook:builtin-post-scan] symbols={} edges={}",
                graph.node_count(),
                graph.edge_count()
            );
            Ok(())
        },
    ));

    reg
}

pub fn run(args: PluginArgs, globals: &crate::global_opts::GlobalOpts) -> anyhow::Result<()> {
    match args.command {
        PluginCommands::List => {
            let reg = default_registry();
            println!("Registered plugin hooks ({}):", reg.len());
            for hook in reg.list() {
                let kind_str = match hook.kind {
                    HookKind::PreAnalysis => "pre",
                    HookKind::PostAnalysis => "post",
                };
                let exec = if hook.callback.is_some() {
                    "executable"
                } else {
                    "metadata-only"
                };
                println!(
                    "  [{kind_str}] {} ({}) — {}",
                    hook.name, exec, hook.description
                );
            }
        }
        PluginCommands::Run { path: _ } => {
            use coregraph_extractor::build_graph_with_hooks;
            let reg = default_registry();
            let (graph, files) = build_graph_with_hooks(&globals.project, &reg)?;
            println!(
                "Hooks fired. Index summary: {} files, {} symbols, {} edges",
                files,
                graph.node_count(),
                graph.edge_count()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_executable_hooks() {
        let reg = default_registry();
        assert!(reg.len() >= 2);
        assert!(reg.contains("builtin-pre-scan"));
        assert!(reg.contains("builtin-post-scan"));
        // All builtins must have callbacks.
        for hook in reg.list() {
            assert!(
                hook.callback.is_some(),
                "hook {} should be executable, not metadata-only",
                hook.name
            );
        }
    }

    #[test]
    fn default_registry_has_pre_and_post_hooks() {
        let reg = default_registry();
        assert!(!reg.list_by_kind(HookKind::PreAnalysis).is_empty());
        assert!(!reg.list_by_kind(HookKind::PostAnalysis).is_empty());
    }

    #[test]
    fn pre_hook_rejects_nonexistent_path() {
        let reg = default_registry();
        let result = reg.fire_pre(std::path::Path::new("/nonexistent/phantom/path/xyz"));
        assert!(result.is_err());
    }
}
