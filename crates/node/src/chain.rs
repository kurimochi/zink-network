use alloy::{
    primitives::{Address, U256},
    rpc::types::Log,
    sol_types::SolEvent,
};
use common::chain::ZinKNetContract;
use std::{collections::HashMap, error::Error};

pub async fn handle_blockchain_event(
    log: Log,
    task_queue: &mut HashMap<U256, Address>,
    elf_queue: &mut HashMap<U256, (Vec<u8>, Address)>,
) -> Result<(), Box<dyn Error>> {
    match log.topic0() {
        Some(&ZinKNetContract::TaskCreated::SIGNATURE_HASH) => {
            let ZinKNetContract::TaskCreated {
                taskId,
                requestor,
                reward,
            } = log.log_decode()?.inner.data;
            println!(
                "TaskCreated: taskId = {:?}, requestor = {:?}, reward = {:?}",
                taskId, requestor, reward
            );

            if elf_queue.contains_key(&taskId) {
                if !elf_queue[&taskId].1.eq(&requestor) {
                    elf_queue.remove(&taskId);
                    task_queue.insert(taskId, requestor);
                    println!(
                        "Signature address mismatch for task {}: expected {}, got {}",
                        taskId, elf_queue[&taskId].1, requestor
                    );
                    return Ok(());
                }
            } else {
                task_queue.insert(taskId, requestor);
                println!("Tasks come first");
            }
        }
        _ => {}
    }
    Ok(())
}
