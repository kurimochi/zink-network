use crate::ReadyCompetition;
use alloy::{
    primitives::{Address, U256},
    providers::Provider,
};
use common::{
    chain::ZinKNet,
    p2p::{Behaviour, BehaviourEvent},
    payload::ElfPayload,
};
use libp2p::{gossipsub, mdns, swarm::SwarmEvent};
use std::{collections::HashMap, error::Error};
use tracing::{info, warn};

pub async fn handle_swarm_event<P: Provider + Send + Sync>(
    event: SwarmEvent<BehaviourEvent>,
    swarm: &mut libp2p::Swarm<Behaviour>,
    zinknet: &ZinKNet<P>,
    chain_only_competitions: &mut HashMap<U256, (Address, U256)>,
    elf_only_competitions: &mut HashMap<U256, (Vec<u8>, Address)>,
    ready_competitions: &mut HashMap<U256, ReadyCompetition>,
) -> Result<(), Box<dyn Error>> {
    match event {
        SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
            message,
            ..
        })) => {
            if let Ok(payload) = bincode::deserialize::<ElfPayload>(&message.data) {
                let competition_id = payload.competition_id;
                info!("Received ELF for competition ID: {}", competition_id);
                if let Ok(elf_signer) = payload.signature.recover_address_from_msg(&payload.elf) {
                    // Check if the corresponding Competition has already arrived
                    if let Some((issuer, reward)) = chain_only_competitions.remove(&competition_id)
                    {
                        // Competition came first. Now we have a pair.
                        if elf_signer != issuer {
                            warn!(
                                "ELF signer mismatch for competition {}. Expected: {}, Got: {}",
                                competition_id, issuer, elf_signer
                            );
                            // Re-insert the competition to pending queue as the ELF was invalid
                            chain_only_competitions.insert(competition_id, (issuer, reward));
                            return Ok(());
                        }

                        // Fetch competitor count from the contract
                        let competitors = zinknet
                            .contract
                            .getCompetitorCount(competition_id)
                            .call()
                            .await?;

                        let new_ready_competition = ReadyCompetition {
                            competition_id,
                            issuer,
                            reward,
                            competitors,
                        };
                        info!("Competition {} is now ready for execution.", competition_id);
                        ready_competitions.insert(competition_id, new_ready_competition);
                    } else {
                        // ELF came first. Add to pending queue.
                        info!(
                            "ELF for competition {} is pending CompetitionOpened event.",
                            competition_id
                        );
                        elf_only_competitions.insert(competition_id, (payload.elf, elf_signer));
                    }
                } else {
                    warn!(
                        "Failed to recover address from ELF signature for competition {}",
                        competition_id
                    );
                }
            } else {
                warn!("Failed to deserialize Gossipsub message.");
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
            for (peer_id, _multiaddr) in list {
                info!("mDNS discovered a new peer: {}", peer_id);
                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
            for (peer_id, _multiaddr) in list {
                info!("mDNS peer has expired: {}", peer_id);
                swarm
                    .behaviour_mut()
                    .gossipsub
                    .remove_explicit_peer(&peer_id);
            }
        }
        _ => {}
    }
    Ok(())
}
