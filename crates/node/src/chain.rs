use alloy::{
    eips::BlockNumberOrTag,
    primitives::{Address, U256},
    providers::Provider,
    rpc::types::{Filter, Log},
    sol,
    sol_types::SolEvent,
};
use std::{collections::HashMap, error::Error, time::Duration};
use tokio::time::sleep;

sol!(
    #[sol(rpc)]
    ZinKNetContract,
    "../../out/zinknet.sol/ZinKNetContract.json"
);

// Use when RPC supports event subscription
// If not, use setup_ethlistener_polling

// pub async fn setup_ethlistener<T>(
//     provider: T,
//     contract_addr: Address,
// ) -> Result<SubscriptionStream<Log>, Box<dyn Error>>
// where
//     T: Provider + Send + Sync + 'static,
// {
//     let filter = Filter::new()
//         .address(contract_addr)
//         .from_block(BlockNumberOrTag::Latest);

//     let sub = provider.subscribe_logs(&filter).await?;
//     Ok(sub.into_stream())
// }

pub async fn setup_ethlistener_polling<T>(
    provider: T,
    contract_addr: Address,
) -> Result<tokio_stream::wrappers::UnboundedReceiverStream<Log>, Box<dyn Error>>
where
    T: Provider + Send + Sync + 'static,
{
    let base_filter = Filter::new().address(contract_addr);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    tokio::spawn(async move {
        let mut last_polled_block: u64 =
            provider.get_block_number().await.unwrap_or_default().into();

        loop {
            sleep(Duration::from_secs(2)).await;

            match provider.get_block_number().await {
                Ok(current_block_num) => {
                    let current_block: u64 = current_block_num.into();
                    if current_block > last_polled_block {
                        let filter = base_filter
                            .clone()
                            .from_block(BlockNumberOrTag::Number(last_polled_block + 1))
                            .to_block(BlockNumberOrTag::Number(current_block));

                        match provider.get_logs(&filter).await {
                            Ok(logs) => {
                                for log in logs {
                                    if tx.send(log).is_err() {
                                        return;
                                    }
                                }
                                last_polled_block = current_block;
                            }
                            Err(e) => {
                                println!("Error fetching logs: {:?}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("Error getting block number: {:?}", e);
                }
            }
        }
    });

    Ok(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
}

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
