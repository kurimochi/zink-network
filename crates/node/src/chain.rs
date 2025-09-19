use crate::ReadyCompetition;
use alloy::{
    primitives::{Address, U256},
    providers::Provider,
    rpc::types::Log,
    sol_types::SolEvent,
};
use common::chain::{ZinKNet, ZinKNetContract};
use sp1_sdk::SP1Stdin;
use std::{collections::HashMap, error::Error};
use tracing::{info, warn};

pub async fn handle_blockchain_event<P: Provider + Send + Sync>(
    log: Log,
    zinknet: &ZinKNet<P>,
    user_addr: Address,
    chain_only_competitions: &mut HashMap<U256, (Address, U256)>,
    elf_only_competitions: &mut HashMap<U256, (Vec<u8>, SP1Stdin, Address)>,
    ready_competitions: &mut HashMap<U256, ReadyCompetition>,
    active_competition_id: &mut Option<U256>,
) -> Result<(), Box<dyn Error>> {
    match log.topic0() {
        // --- CompetitionOpened Event ---
        Some(&ZinKNetContract::CompetitionOpened::SIGNATURE_HASH) => {
            let decoded = log.log_decode::<ZinKNetContract::CompetitionOpened>()?;
            let ZinKNetContract::CompetitionOpened {
                competitionId,
                issuer,
                reward,
            } = decoded.inner.data;
            info!(
                "CompetitionOpened event received for competition ID: {}, Issuer: {}, Reward: {}",
                competitionId, issuer, reward
            );

            // Check if the corresponding ELF has already arrived
            if let Some((elf_bytes, stdin, elf_signer)) =
                elf_only_competitions.remove(&competitionId)
            {
                // ELF came first. Now we have a pair.
                if elf_signer != issuer {
                    warn!(
                        "ELF signer mismatch for competition {}. Expected: {}, Got: {}",
                        competitionId, issuer, elf_signer
                    );
                    return Ok(());
                }

                // Fetch competitor count from the contract
                let competitors = zinknet
                    .contract
                    .getCompetitorCount(competitionId)
                    .call()
                    .await?;

                let new_ready_competition = ReadyCompetition {
                    competition_id: competitionId,
                    issuer,
                    reward,
                    competitors,
                    elf: elf_bytes,
                    stdin,
                };
                info!("Competition {} is now ready for execution.", competitionId);
                ready_competitions.insert(competitionId, new_ready_competition);
            } else {
                // Competition came first. Add to pending queue.
                info!("Competition {} is pending ELF.", competitionId);
                chain_only_competitions.insert(competitionId, (issuer, reward));
            }
        }

        // --- CompetitionJoined Event ---
        Some(&ZinKNetContract::CompetitionJoined::SIGNATURE_HASH) => {
            let decoded = log.log_decode::<ZinKNetContract::CompetitionJoined>()?;
            let ZinKNetContract::CompetitionJoined {
                competitionId,
                competitor,
            } = decoded.inner.data;

            if let Some(competition) = ready_competitions.get_mut(&competitionId) {
                competition.competitors += U256::from(1);
                info!(
                    "CompetitionJoined event for competition {}. New count: {}. Competitor: {}",
                    competitionId, competition.competitors, competitor
                );
            }

            if user_addr == competitor {
                *active_competition_id = Some(competitionId);
                info!("You have joined the competition {}.", competitionId);
            }
        }

        // --- CompetitionLeft Event ---
        Some(&ZinKNetContract::CompetitionLeft::SIGNATURE_HASH) => {
            let decoded = log.log_decode::<ZinKNetContract::CompetitionLeft>()?;
            let ZinKNetContract::CompetitionLeft {
                competitionId,
                competitor,
            } = decoded.inner.data;

            if let Some(competition) = ready_competitions.get_mut(&competitionId) {
                if competition.competitors > U256::ZERO {
                    competition.competitors -= U256::from(1);
                }
                info!(
                    "CompetitionLeft event for competition {}. New count: {}. Competitor: {}",
                    competitionId, competition.competitors, competitor
                );
            }
            if competitor == user_addr {
                *active_competition_id = None;
                info!("You have left the competition {}.", competitionId);
            }
        }
        _ => {}
    }
    Ok(())
}
