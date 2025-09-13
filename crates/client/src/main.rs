use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::{SignerSync, local::PrivateKeySigner},
    sol,
    transports::http::reqwest::Url,
};
use clap::Parser;
use libp2p::{
    futures::StreamExt,
    gossipsub::{self, IdentTopic},
    mdns, noise,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    tcp, yamux,
};
use std::{
    env,
    error::Error,
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

sol!(
    #[sol(rpc)]
    ZinKNetContract,
    "../../out/zinknet.sol/ZinKNetContract.json"
);

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
}

impl Behaviour {
    pub fn new(gossipsub: gossipsub::Behaviour, mdns: mdns::tokio::Behaviour) -> Self {
        Self { gossipsub, mdns }
    }
}

pub fn setup_gossipsub(
    topic_name: &str,
) -> Result<(libp2p::Swarm<Behaviour>, gossipsub::IdentTopic), Box<dyn Error>> {
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|keypair| {
            let message_id_fn = |message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            };

            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(10))
                .validation_mode(gossipsub::ValidationMode::Strict)
                .message_id_fn(message_id_fn)
                .build()
                .map_err(std::io::Error::other)?;

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(keypair.clone()),
                gossipsub_config,
            )?;

            let mdns =
                mdns::tokio::Behaviour::new(mdns::Config::default(), keypair.public().into())?;
            Ok(Behaviour::new(gossipsub, mdns))
        })?
        .build();

    let topic = gossipsub::IdentTopic::new(topic_name);
    swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

    Ok((swarm, topic))
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    private_key_name: String,

    #[arg(short, long)]
    reward: String,
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

async fn create_onchain_task<P: Provider + Clone>(
    signer: &PrivateKeySigner,
    provider: &P,
    reward: U256,
) -> Result<U256, Box<dyn Error>> {
    println!("Creating task on blockchain...");
    let contract_address: Address = env::var("CONTRACT_ADDRESS")?.parse()?;
    let contract = ZinKNetContract::new(contract_address, provider);

    let verification_gas = contract.verificationGas().call().await?;
    let receipt = contract
        .createTask(reward)
        .value(reward + verification_gas)
        .from(signer.address())
        .send()
        .await?
        .get_receipt()
        .await?;

    let taskcreated_log = receipt
        .decoded_log::<ZinKNetContract::TaskCreated>()
        .ok_or("Failed to find or decode TaskCreated log")?;

    let task_id = taskcreated_log.taskId;
    println!("Created task with ID: {}", task_id);
    Ok(task_id)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv::dotenv().ok();

    let cli = Cli::parse();
    let signer: PrivateKeySigner = env::var(cli.private_key_name)?.parse()?;
    let reward = U256::from_str_radix(&cli.reward, 10)?;
    let rpc_url = Url::parse(&env::var("HTTP_RPC_URL")?)?;
    let provider = ProviderBuilder::new().connect_http(rpc_url);

    let (mut swarm, topic) = setup_gossipsub("test")?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    find_subscribed_peer(&mut swarm, &topic).await?;

    let task_id = create_onchain_task(&signer, &provider, reward).await?;
    // Uncomment tokio::time::sleep if you want to send TaskCreated to node first
    // tokio::time::sleep(Duration::from_secs(5)).await;

    println!("Preparing and publishing payload...");
    let test_binary = b"Hello, world!";
    let signature = signer.sign_message_sync(test_binary)?;
    let elf_payload = (task_id, test_binary.to_vec(), signature);

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
