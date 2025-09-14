use alloy::{
    primitives::{Address, U256},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    transports::ws::WsConnect,
};
use clap::Parser;
use common::{
    chain::ZinKNet,
    config::CommonConfig,
    p2p::{Behaviour, SwarmExt},
};
use dotenv::dotenv;
use libp2p::futures::StreamExt;
use std::{collections::HashMap, error::Error};
use tokio::select;

mod chain;
mod p2p;

use chain::handle_blockchain_event;
use p2p::handle_swarm_event;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    common: CommonConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    let cli = Cli::parse();

    // Setup Signer
    let signer: PrivateKeySigner = cli.common.private_key.parse()?;
    let user_addr = signer.address();
    println!("Your address: {}", user_addr);

    // P2P Setup
    let mut swarm = Behaviour::new_swarm()?;
    let _ = swarm.subscribe("test")?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Blockchain Setup
    let ws = WsConnect::new(&cli.common.rpc_url);
    let provider = ProviderBuilder::new().connect_ws(ws).await?;
    let zinknet = ZinKNet::new(provider, cli.common.contract);
    let mut stream = zinknet.setup_ethlistener_polling().await?;

    // Task and ELF Queues
    let mut task_queue = HashMap::<U256, Address>::new();
    let mut elf_queue = HashMap::<U256, (Vec<u8>, Address)>::new();

    println!("Starting event loop...");

    loop {
        select! {
            Some(log) = stream.next() => {
                if let Err(e) = handle_blockchain_event(log, &mut task_queue, &mut elf_queue).await {
                    println!("Error handling blockchain event: {:?}", e);
                }
            },
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &mut swarm, &mut task_queue, &mut elf_queue)?;
            }
        }
    }
}
