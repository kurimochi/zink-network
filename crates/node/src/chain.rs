use crate::ReadyTask;
use alloy::{
    primitives::{Address, U256},
    providers::Provider,
    rpc::types::Log,
    sol_types::SolEvent,
};
use common::chain::{ZinKNet, ZinKNetContract};
use std::{collections::HashMap, error::Error};

pub async fn handle_blockchain_event<P: Provider + Send + Sync>(
    log: Log,
    zinknet: &ZinKNet<P>,
    pending_tasks: &mut HashMap<U256, (Address, U256)>,
    pending_elfs: &mut HashMap<U256, (Vec<u8>, Address)>,
    ready_tasks: &mut HashMap<U256, ReadyTask>,
) -> Result<(), Box<dyn Error>> {
    match log.topic0() {
        // --- TaskCreated Event ---
        Some(&ZinKNetContract::TaskCreated::SIGNATURE_HASH) => {
            let decoded = log.log_decode::<ZinKNetContract::TaskCreated>()?;
            let ZinKNetContract::TaskCreated {
                taskId,
                requestor,
                reward,
            } = decoded.inner.data;

            // Check if the corresponding ELF has already arrived
            if let Some((_elf_bytes, elf_signer)) = pending_elfs.remove(&taskId) {
                // ELF came first. Now we have a pair.
                if elf_signer != requestor {
                    // TODO: Handle address mismatch. For now, just log and ignore.
                    return Ok(());
                }

                // Fetch declaration count from the contract
                let declarations = zinknet.contract.getDeclarationCount(taskId).call().await?;

                let new_ready_task = ReadyTask {
                    task_id: taskId,
                    requestor,
                    reward,
                    declarations,
                };
                ready_tasks.insert(taskId, new_ready_task);
                // TODO: Log that a task is ready
            } else {
                // Task came first. Add to pending queue.
                pending_tasks.insert(taskId, (requestor, reward));
                // TODO: Log that a task is pending
            }
        }

        // --- WorkDeclared Event ---
        Some(&ZinKNetContract::WorkDeclared::SIGNATURE_HASH) => {
            let decoded = log.log_decode::<ZinKNetContract::WorkDeclared>()?;
            let ZinKNetContract::WorkDeclared { taskId, prover: _ } = decoded.inner.data;

            if let Some(task) = ready_tasks.get_mut(&taskId) {
                task.declarations += U256::from(1);
                // TODO: Log declaration increment
            }
        }

        // --- DeclarationCancelled Event ---
        Some(&ZinKNetContract::DeclarationCancelled::SIGNATURE_HASH) => {
            let decoded = log.log_decode::<ZinKNetContract::DeclarationCancelled>()?;
            let ZinKNetContract::DeclarationCancelled { taskId, prover: _ } = decoded.inner.data;

            if let Some(task) = ready_tasks.get_mut(&taskId) {
                if task.declarations > U256::ZERO {
                    task.declarations -= U256::from(1);
                }
                // TODO: Log declaration decrement
            }
        }
        _ => {}
    }
    Ok(())
}
