use libp2p::{gossipsub, mdns, noise, swarm::NetworkBehaviour, tcp, yamux};
use std::{
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

    pub fn new_swarm() -> Result<libp2p::Swarm<Self>, Box<dyn Error>> {
        let swarm = libp2p::SwarmBuilder::with_new_identity()
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
        Ok(swarm)
    }
}

pub trait SwarmExt {
    fn subscribe(&mut self, topic_name: &str) -> Result<gossipsub::IdentTopic, Box<dyn Error>>;
}

impl SwarmExt for libp2p::Swarm<Behaviour> {
    fn subscribe(&mut self, topic_name: &str) -> Result<gossipsub::IdentTopic, Box<dyn Error>> {
        let topic = gossipsub::IdentTopic::new(topic_name);
        self.behaviour_mut().gossipsub.subscribe(&topic)?;
        Ok(topic)
    }
}
