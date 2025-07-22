use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kailua_common::client::soon_test::{soon_to_derivation, soon_to_execution_cache};
use kona_cli::init_tracing_subscriber;
use kona_proof::BootInfo;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// output directory for oracle storage (oracle data will be stored in this directory)
    #[arg(short, long)]
    output: PathBuf,

    /// output file path for offline config
    #[arg(short, long)]
    config: Option<PathBuf>,

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

fn generate_offline_config(
    boot_info: &BootInfo,
    oracle_dir_path: &PathBuf,
    config_file_path: &PathBuf,
) -> Result<()> {
    // Calculate relative path from config file to oracle directory
    let config_dir = config_file_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    // Simple relative path calculation - if oracle dir is in same directory as config, just use directory name
    let relative_oracle_path = if oracle_dir_path.parent() == Some(config_dir) {
        oracle_dir_path
            .file_name()
            .unwrap_or(oracle_dir_path.as_os_str())
            .to_string_lossy()
            .to_string()
    } else {
        oracle_dir_path.to_string_lossy().to_string()
    };

    let config = json!({
        "boot_info": {
            "l1_head": format!("0x{:064x}", boot_info.l1_head),
            "agreed_l2_output_root": format!("0x{:064x}", boot_info.agreed_l2_output_root),
            "claimed_l2_output_root": format!("0x{:064x}", boot_info.claimed_l2_output_root),
            "claimed_l2_block_number": boot_info.claimed_l2_block_number,
            "agreed_l2_block_number": boot_info.agreed_l2_block_number,
            "chain_id": boot_info.chain_id,
            "rollup_config": boot_info.rollup_config
        },
        "source_db_path": relative_oracle_path,
        "target_db_path": "/tmp/kailua_offline_test",
        "precondition_validation_data": null,
        "analysis": true,
        "native_client": true,
        "offchain_oracle": true
    });

    let config_content =
        serde_json::to_string_pretty(&config).context("Failed to serialize offline config")?;

    fs::write(config_file_path, config_content).context("Failed to write offline config file")?;

    Ok(())
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

            let (boot_info, oracle) = soon_to_execution_cache(Some("../../.."))
                .await
                .context("Failed to generate execution cache")?;

            info!(
                "Successfully generated execution cache: {} execution records, L2 block number: {}",
                oracle.executions.len(),
                boot_info.claimed_l2_block_number
            );

            if *verbose {
                info!("Boot info: {:?}", boot_info);
                info!("Execution record count: {}", oracle.executions.len());
            }

            // Save oracle data to a file within the output directory
            oracle
                .write_to_disk(&cli.output)
                .context("Failed to write oracle storage to disk")?;

            // Generate offline config if requested
            if let Some(config_path) = &cli.config {
                generate_offline_config(&boot_info, &cli.output, config_path)
                    .context("Failed to generate offline config")?;
                info!("Offline config saved to: {:?}", config_path);
            }
        }
        Commands::Derivation { verbose } => {
            info!("Start generating MockOracle storage using soon_to_derivation...");

            if *verbose {
                info!("verbose mode enabled");
            }

            let (boot_info, oracle) = soon_to_derivation(Some("../../.."))
                .await
                .context("Failed to generate derivation")?;

            info!(
                "Successfully generated derivation data, L2 block number: {}",
                boot_info.claimed_l2_block_number
            );

            if *verbose {
                info!("Boot info: {:?}", boot_info);
            }

            // Save oracle data to a file within the output directory
            oracle
                .write_to_disk(&cli.output)
                .context("Failed to write oracle storage to disk")?;

            // Generate offline config if requested
            if let Some(config_path) = &cli.config {
                generate_offline_config(&boot_info, &cli.output, config_path)
                    .context("Failed to generate offline config")?;
                info!("Offline config saved to: {:?}", config_path);
            }
        }
    }

    info!("Tool execution completed!");
    Ok(())
}
