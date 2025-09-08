use alloy::{providers::ProviderBuilder, transports::ws::WsConnect};
use clap::Parser;
use libp2p::futures::StreamExt;
use std::error::Error;
use tokio::{select, sync::mpsc};

mod chain;
mod config;
mod p2p;

use chain::{Command, handle_blockchain_event, setup_ethlistener_polling};
use config::{Config, get_signer_from_env};
use p2p::{handle_swarm_event, setup_gossipsub};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// The environment variable name for the private key.
    #[arg(short, long)]
    private_key_name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    // Setup Configuration & Signer
    let config = Config::from_env()?;
    let signer = get_signer_from_env(&cli.private_key_name)?;
    let user_addr = signer.address();
    println!("Using private key from env: {}", cli.private_key_name);
    println!("Your address: {}", user_addr);

    // P2P Setup
    let (mut swarm, topic) = setup_gossipsub("test")?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Blockchain Setup
    let ws = WsConnect::new(&config.rpc_url);
    let provider = ProviderBuilder::new().connect_ws(ws).await?;
    let mut stream = setup_ethlistener_polling(provider.clone(), config.contract_address).await?;

    // Command channel for internal communication
    let (command_sender, mut command_receiver) = mpsc::unbounded_channel::<Command>();

    println!("Starting event loop...");

    loop {
        select! {
            Some(command) = command_receiver.recv() => {
                match command {
                    Command::Publish(message) => {
                        if let Err(e) = swarm
                            .behaviour_mut().gossipsub
                            .publish(topic.clone(), message.as_bytes())
                        {
                            println!("Publish error: {:?}", e);
                        }
                    }
                }
            },
            Some(log) = stream.next() => {
                let command_sender_clone = command_sender.clone();
                if let Err(e) = handle_blockchain_event(log, user_addr, provider.clone(), &signer, config.contract_address, command_sender_clone).await {
                    println!("Error handling blockchain event: {:?}", e);
                }
            },
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &mut swarm);
            }
        }
    }
}
