use alloy::{
    primitives::U256,
    providers::ProviderBuilder,
    signers::{SignerSync, local::PrivateKeySigner},
    transports::http::reqwest::Url,
};
use clap::Parser;
use common::{
    chain::ZinKNet,
    config::CommonConfig,
    p2p::{Behaviour, BehaviourEvent, SwarmExt},
    payload::ElfPayload,
};
use libp2p::{
    futures::StreamExt,
    gossipsub::IdentTopic,
    mdns,
    swarm::{Swarm, SwarmEvent},
};
use std::{error::Error, time::Duration};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    common: CommonConfig,
    #[arg(long)]
    reward: String,
    #[arg(long, value_name = "ELF_PATH", env = "ELF_PATH")]
    elf: String,
}

async fn find_subscribed_peer(
    swarm: &mut Swarm<Behaviour>,
    topic: &IdentTopic,
) -> Result<(), Box<dyn Error>> {
    println!("Searching for peers subscribed to topic '{}'...", topic);
    let topic_hash_to_check = topic.hash();
    loop {
        let subscribed_peer_exists = swarm
            .behaviour()
            .gossipsub
            .all_peers()
            .any(|(_peer_id, topics)| topics.contains(&&topic_hash_to_check));

        if subscribed_peer_exists {
            println!("Found peer subscribed to the topic. Proceeding...");
            return Ok(());
        }

        match swarm.select_next_some().await {
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                for (peer_id, _multiaddr) in list {
                    println!("mDNS discovered a new peer: {}", peer_id);
                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                }
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv::dotenv().ok();

    let cli = Cli::parse();
    let signer: PrivateKeySigner = cli.common.private_key.parse()?;
    let reward = U256::from_str_radix(&cli.reward, 10)?;
    let rpc_url = Url::parse(&cli.common.rpc_url)?;
    let provider = ProviderBuilder::new().connect_http(rpc_url);

    let mut swarm = Behaviour::new_swarm()?;
    let topic = swarm.subscribe("test")?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    find_subscribed_peer(&mut swarm, &topic).await?;

    let zinknet = ZinKNet::new(provider, cli.common.contract);
    let competition_id = zinknet.open_competition(&signer, reward).await?;

    // Uncomment tokio::time::sleep if you want to send CompetitionOpened to node first
    // tokio::time::sleep(Duration::from_secs(5)).await;

    println!("Preparing and publishing payload...");
    let elf = std::fs::read(cli.elf)?;
    let signature = signer.sign_message_sync(&elf)?;
    let elf_payload = ElfPayload {
        competition_id,
        elf,
        signature,
    };

    let payload_bin = bincode::serialize(&elf_payload)?;

    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.clone(), payload_bin)?;

    println!("Payload published to gossipsub topic.");

    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("Client finished its job and is now exiting.");

    Ok(())
}
