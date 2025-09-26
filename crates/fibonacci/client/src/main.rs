use alloy::{
    primitives::U256, providers::ProviderBuilder, signers::local::PrivateKeySigner,
    transports::http::reqwest::Url,
};
use clap::Parser;
use client_lib::CompetitionManager;
use common::{chain::ZinKNet, config::CommonConfig};
use sp1_sdk::SP1Stdin;
use std::{error::Error, time::Duration};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    common: CommonConfig,
    #[arg(long)]
    elf_path: String,
    #[arg(long)]
    nth: u32,
    #[arg(long)]
    reward: U256,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv::dotenv().ok();

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let cli = Cli::parse();
    let rpc_url = Url::parse(&cli.common.rpc_url)?;
    let provider = ProviderBuilder::new().connect_http(rpc_url);

    let zinknet = ZinKNet::new(
        provider,
        cli.common.contract,
        cli.common.private_key.parse::<PrivateKeySigner>()?,
    );

    let mut competition_manager = CompetitionManager::new(zinknet, "zinknet-elf").await?;

    competition_manager.find_subscribed_peer().await?;

    let elf = std::fs::read(cli.elf_path)?;
    let mut stdin = SP1Stdin::new();
    stdin.write(&cli.nth);

    competition_manager
        .open_competition_and_publish_payload(cli.reward, elf, stdin)
        .await?;

    tokio::time::sleep(Duration::from_secs(1)).await;

    info!("Fibonacci client finished its job and is now exiting.");

    Ok(())
}
