use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kailua_common::client::soon_test::{soon_to_derivation, soon_to_execution_cache};
use kona_cli::init_tracing_subscriber;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// output file path
    #[arg(short, long)]
    output: PathBuf,

    /// log level
    #[arg(short, long, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    /// generate MockOracle storage using soon_to_execution_cache
    ExecutionCache {
        /// verbose mode
        #[arg(short, long)]
        verbose: bool,
    },
    /// generate MockOracle storage using soon_to_derivation
    Derivation {
        /// verbose mode
        #[arg(short, long)]
        verbose: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // initialize logging
    let log_level = match cli.log_level.to_lowercase().as_str() {
        "trace" => 5,
        "debug" => 4,
        "info" => 3,
        "warn" => 2,
        "error" => 1,
        _ => 3,
    };
    init_tracing_subscriber(log_level, None::<EnvFilter>)
        .context("Failed to initialize tracing subscriber")?;

    match &cli.command {
        Commands::ExecutionCache { verbose } => {
            info!("Start generating MockOracle storage using soon_to_execution_cache...");

            if *verbose {
                info!("verbose mode enabled");
            }

            let (boot_info, executions, oracle) = soon_to_execution_cache()
                .await
                .context("Failed to generate execution cache")?;

            info!(
                "Successfully generated execution cache: {} execution records, L2 block number: {}",
                executions.len(),
                boot_info.claimed_l2_block_number
            );

            if *verbose {
                info!("Boot info: {:?}", boot_info);
                info!("Execution record count: {}", executions.len());
            }

            oracle
                .write_to_disk(&cli.output)
                .context("Failed to write oracle storage to disk")?;

            info!("MockOracle storage saved to: {:?}", cli.output);
        }
        Commands::Derivation { verbose } => {
            info!("Start generating MockOracle storage using soon_to_derivation...");

            if *verbose {
                info!("verbose mode enabled");
            }

            let (boot_info, oracle) = soon_to_derivation()
                .await
                .context("Failed to generate derivation")?;

            info!(
                "Successfully generated derivation data, L2 block number: {}",
                boot_info.claimed_l2_block_number
            );

            if *verbose {
                info!("Boot info: {:?}", boot_info);
            }

            oracle
                .write_to_disk(&cli.output)
                .context("Failed to write oracle storage to disk")?;

            info!("MockOracle storage saved to: {:?}", cli.output);
        }
    }

    info!("Tool execution completed!");
    Ok(())
}
