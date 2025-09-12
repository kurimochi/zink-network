use alloy::{
    primitives::{Address, U256},
    signers::Signature,
};
use libp2p::{
    gossipsub, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use std::{
    collections::HashMap,
    error::Error,
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};
use tokio::io;

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
        .with_tcp(tcp::Config::default(), noise::Config::new, || {
            yamux::Config::default()
        })?
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
                .map_err(io::Error::other)?;

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

pub fn handle_swarm_event(
    event: SwarmEvent<BehaviourEvent>,
    swarm: &mut libp2p::Swarm<Behaviour>,
    task_queue: &mut HashMap<U256, Address>,
    elf_queue: &mut HashMap<U256, (Vec<u8>, Address)>,
) -> Result<(), Box<dyn Error>> {
    match event {
        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
            for (peer_id, _multiaddr) in list {
                println!("mDNS discovered a new peer: {}", peer_id);
                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
            for (peer_id, _multiaddr) in list {
                println!("mDNS discover peer has expired: {}", peer_id);
                swarm
                    .behaviour_mut()
                    .gossipsub
                    .remove_explicit_peer(&peer_id);
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source: peer_id,
            message_id: _,
            message,
        })) => {
            let (task_id, elf, signature): (U256, Vec<u8>, Signature) =
                bincode::deserialize(&message.data)?;
            let recover_address = signature.recover_address_from_msg(&elf)?;
            if task_queue.contains_key(&task_id) {
                if recover_address != task_queue[&task_id] {
                    println!(
                        "Signature address mismatch for task {}: expected {}, got {}",
                        task_id, task_queue[&task_id], recover_address
                    );
                    return Ok(());
                }
                println!(
                    "Received valid ELF for task {} from peer {}",
                    task_id, peer_id
                );
                task_queue.remove(&task_id);
            } else {
                elf_queue.insert(task_id, (elf, recover_address));
                println!("ELF come first");
            }
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            println!("Local node is listening on {}", address);
        }
        _ => {}
    }
    Ok(())
}
