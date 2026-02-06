mod commands;
mod github;
mod models;
mod parsers;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "gh-logs-grab")]
#[command(about = "Download and analyze GitHub Actions logs blazingly fast")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download logs from a GitHub Actions run
    Download {
        /// GitHub Actions run URL (e.g., https://github.com/owner/repo/actions/runs/123456)
        /// Can also be a job URL - will extract the run ID
        run_url: String,

        /// Output directory for logs (defaults to ./logs)
        #[arg(short, long, default_value = "logs")]
        output: PathBuf,

        /// GitHub token (reads from GITHUB_TOKEN env var if not provided)
        #[arg(short, long, env = "GITHUB_TOKEN")]
        token: Option<String>,

        /// Download all logs (by default only failed logs are downloaded)
        #[arg(short, long)]
        all: bool,
    },
    /// Analyze logs in a run directory to extract test errors (pytest, Jest, Storybook)
    Analyze {
        /// Path to run directory (e.g., logs/pr-123/19374816456)
        run_dir: PathBuf,
    },
    /// Generate timeline analysis across multiple runs
    Timeline {
        /// Path to PR directory (e.g., logs/pr-123)
        pr_dir: PathBuf,
    },
    /// Generate timing analysis across multiple runs
    Timings {
        /// Path to PR directory (e.g., logs/pr-123)
        pr_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Download {
            run_url,
            output,
            token,
            all,
        } => commands::download_command(run_url, output, token, all).await,
        Commands::Analyze { run_dir } => commands::analyze_command(run_dir).await,
        Commands::Timeline { pr_dir } => commands::timeline_command(pr_dir).await,
        Commands::Timings { pr_dir } => commands::timings_command(pr_dir).await,
    }
}
