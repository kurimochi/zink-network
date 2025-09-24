use std::error::Error;

use alloy::{primitives::U256, providers::Provider, signers::Signer};
use common::{
    chain::ZinKNet,
    p2p::{Behaviour, BehaviourEvent, SwarmExt},
    payload::ElfPayload,
};
use libp2p::{
    futures::StreamExt,
    gossipsub::IdentTopic,
    mdns,
    swarm::{Swarm, SwarmEvent},
};
use log::info;
use sp1_sdk::SP1Stdin;

pub struct CompetitionManager<P, S> {
    zinknet: ZinKNet<P, S>,
    swarm: Swarm<Behaviour>,
    topic: IdentTopic,
}

impl<P, S> CompetitionManager<P, S>
where
    P: Provider + Clone,
    S: Signer + Sync + Clone,
{
    pub async fn new(zinknet: ZinKNet<P, S>, topic_name: &str) -> Result<Self, Box<dyn Error>> {
        let mut swarm = Behaviour::new_swarm()?;
        let topic = swarm.subscribe(topic_name)?;
        swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

        Ok(Self {
            zinknet,
            swarm,
            topic,
        })
    }

    pub async fn find_subscribed_peer(&mut self) -> Result<(), Box<dyn Error>> {
        info!(
            "Searching for peers subscribed to topic '{}' ...",
            self.topic
        );
        let topic_hash_to_check = self.topic.hash();
        loop {
            let subscribed_peer_exists = self
                .swarm
                .behaviour()
                .gossipsub
                .all_peers()
                .any(|(_peer_id, topics)| topics.contains(&&topic_hash_to_check));

            if subscribed_peer_exists {
                info!("Found peer subscribed to the topic. Proceeding...");
                return Ok(());
            }

            if let SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) =
                self.swarm.select_next_some().await
            {
                for (peer_id, _multiaddr) in list {
                    info!("mDNS discovered a new peer: {}", peer_id);
                    self.swarm
                        .behaviour_mut()
                        .gossipsub
                        .add_explicit_peer(&peer_id);
                }
            }
        }
    }

    pub async fn open_competition_and_publish_payload(
        &mut self,
        reward: U256,
        elf: Vec<u8>,
        stdin: SP1Stdin,
    ) -> Result<(), Box<dyn Error>> {
        let competition_id = self.zinknet.open_competition(reward).await?;

        info!("Preparing and publishing payload...");
        let signature = self.zinknet.signer.sign_message(&elf).await?;
        let elf_payload = ElfPayload {
            competition_id,
            elf,
            stdin,
            signature,
        };

        let payload_bin = bincode::serialize(&elf_payload)?;

        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.topic.clone(), payload_bin)?;

        info!("Payload published to gossipsub topic.");

        Ok(())
    }
}
