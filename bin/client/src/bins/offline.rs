use clap::Parser;
use kailua_client::args::OfflineClientArgs;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Starting offline client");
    let args = OfflineClientArgs::parse();
    kona_cli::init_tracing_subscriber(args.kailua_verbosity, None::<EnvFilter>)?;
    info!("Initialized tracing subscriber");

    kailua_client::offline::run_offline_client(args.offline_cfg)?;
    info!("Offline client finished");
    Ok(())
}
