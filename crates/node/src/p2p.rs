use crate::ReadyTask;
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
    pending_tasks: &mut HashMap<U256, (Address, U256)>,
    pending_elfs: &mut HashMap<U256, (Vec<u8>, Address)>,
    ready_tasks: &mut HashMap<U256, ReadyTask>,
) -> Result<(), Box<dyn Error>> {
    match event {
        SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
            message,
            ..
        })) => {
            if let Ok(payload) = bincode::deserialize::<ElfPayload>(&message.data) {
                let task_id = payload.task_id;
                info!("Received ELF for task ID: {}", task_id);
                if let Ok(elf_signer) = payload.signature.recover_address_from_msg(&payload.elf) {
                    // Check if the corresponding Task has already arrived
                    if let Some((requestor, reward)) = pending_tasks.remove(&task_id) {
                        // Task came first. Now we have a pair.
                        if elf_signer != requestor {
                            warn!(
                                "ELF signer mismatch for task {}. Expected: {}, Got: {}",
                                task_id, requestor, elf_signer
                            );
                            // Re-insert the task to pending queue as the ELF was invalid
                            pending_tasks.insert(task_id, (requestor, reward));
                            return Ok(());
                        }

                        // Fetch declaration count from the contract
                        let declarations =
                            zinknet.contract.getDeclarationCount(task_id).call().await?;

                        let new_ready_task = ReadyTask {
                            task_id,
                            requestor,
                            reward,
                            declarations,
                        };
                        info!("Task {} is now ready for execution.", task_id);
                        ready_tasks.insert(task_id, new_ready_task);
                    } else {
                        // ELF came first. Add to pending queue.
                        info!("ELF for task {} is pending TaskCreated event.", task_id);
                        pending_elfs.insert(task_id, (payload.elf, elf_signer));
                    }
                } else {
                    warn!("Failed to recover address from ELF signature for task {}", task_id);
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
