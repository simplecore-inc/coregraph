use crate::global_opts::GlobalOpts;
use clap::{Args, Subcommand};
use coregraph_extractor::build_graph;
use coregraph_graph::{load_snapshot, save_snapshot, GraphEpoch};
use std::path::PathBuf;

#[derive(Args)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    pub command: SnapshotCommand,
}

#[derive(Subcommand)]
pub enum SnapshotCommand {
    /// Index the project and save a binary snapshot to disk.
    Save {
        /// Output snapshot file path
        #[arg(long, short = 'o')]
        out: PathBuf,
    },
    /// Load a snapshot from disk and print its summary.
    Load {
        /// Snapshot file to load
        file: PathBuf,
    },
}

pub fn run(args: SnapshotArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    match args.command {
        SnapshotCommand::Save { out } => {
            let (graph, file_count) = build_graph(&globals.project)?;
            let epoch = GraphEpoch::zero().next();
            save_snapshot(&out, &graph, epoch, std::time::SystemTime::now())?;
            println!(
                "Saved snapshot: {} files → {} symbols, {} edges → {}",
                file_count,
                graph.node_count(),
                graph.edge_count(),
                out.display()
            );
        }
        SnapshotCommand::Load { file } => {
            let (graph, epoch, _built_at) = load_snapshot(&file)?;
            println!(
                "Loaded snapshot: epoch {} — {} symbols, {} edges (from {})",
                epoch.0,
                graph.node_count(),
                graph.edge_count(),
                file.display()
            );
        }
    }
    Ok(())
}
