use libp2p::{gossipsub, mdns, noise, swarm::{NetworkBehaviour, SwarmEvent}, tcp, yamux};
use std::{error::Error, hash::{DefaultHasher, Hash, Hasher}, time::Duration};
use tokio::io;

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
}

impl Behaviour {
    pub fn new(
        gossipsub: gossipsub::Behaviour,
        mdns: mdns::tokio::Behaviour,
    ) -> Self {
        Self {
            gossipsub,
            mdns,
        }
    }
}

pub fn setup_gossipsub(topic_name: &str) -> Result<(libp2p::Swarm<Behaviour>, gossipsub::IdentTopic), Box<dyn Error>> {
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            || yamux::Config::default(),
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
                .map_err(io::Error::other)?;

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(keypair.clone()),
                gossipsub_config,
            )?;

            let mdns = mdns::tokio::Behaviour::new(
                mdns::Config::default(),
                keypair.public().into(),
            )?;
            Ok(Behaviour::new(gossipsub, mdns))
        })?
        .build();

    let topic = gossipsub::IdentTopic::new(topic_name);
    swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

    Ok((swarm, topic))
}

pub fn handle_swarm_event(event: SwarmEvent<BehaviourEvent>, swarm: &mut libp2p::Swarm<Behaviour>) {
    match event {
        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
            for (peer_id, _multiaddr) in list {
                println!("mDNS discovered a new peer: {}", peer_id);
                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
            }
        },
        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
            for (peer_id, _multiaddr) in list {
                println!("mDNS discover peer has expired: {}", peer_id);
                swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
            }
        },
        SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source: peer_id,
            message_id: id,
            message,
        })) => println!(
                "Got message: '{}' with id: {} from peer: {}",
                String::from_utf8_lossy(&message.data),
                id,
                peer_id,
            ),
        SwarmEvent::NewListenAddr { address, .. } => {
            println!("Local node is listening on {}", address);
        }
        _ => {}
    }
}
