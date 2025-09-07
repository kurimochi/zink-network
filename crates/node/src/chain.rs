use std::{error::Error, time::Duration};
use alloy::{eips::BlockNumberOrTag, primitives::{Address, U256}, providers::Provider, /*pubsub::SubscriptionStream,*/ rpc::types::{Filter, Log}, signers::{local::PrivateKeySigner, Signer}, sol, sol_types::{eip712_domain, SolEvent}};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{io::{self, AsyncBufReadExt}, sync::mpsc, time::sleep};
use std::io::Write;

sol!(
    #[sol(rpc)]
    ZINKNET,
    "../../out/zinknet.sol/ZinKNetContract.json"
);

sol!(
    #[derive(Serialize, Deserialize)]
    struct Bid {
        uint256 taskId;
        uint256 bidAmount;
        address bidder;
    }
);

pub enum Command {
    Publish(String),
}

// Use when RPC supports event subscription
// If not, use setup_ethlistener_polling

// pub async fn setup_ethlistener<T>(provider: T, contract_addr: Address) -> Result<SubscriptionStream<Log>, Box<dyn Error>>
// where
//     T: Provider + Send + Sync + 'static,
// {
//     let filter = Filter::new()
//         .address(contract_addr)
//         .from_block(BlockNumberOrTag::Latest);

//     let sub = provider.subscribe_logs(&filter).await?;
//     Ok(sub.into_stream())
// }

pub async fn setup_ethlistener_polling<T>(provider: T, contract_addr: Address) -> Result<tokio_stream::wrappers::UnboundedReceiverStream<Log>, Box<dyn Error>>
where
    T: Provider + Send + Sync + 'static,
{
    let base_filter = Filter::new().address(contract_addr);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    tokio::spawn(async move {
        let mut last_polled_block: u64 = provider.get_block_number().await.unwrap_or_default().into();

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


async fn handle_task_created(
    task_id: U256,
    user_addr: Address,
    signer: &PrivateKeySigner,
    provider: &(impl Provider + Send + Sync),
    contract_addr: Address,
    command_sender: mpsc::UnboundedSender<Command>,
) -> Result<(), Box<dyn Error>> {
    let domain = eip712_domain! {
        name: "ZinK Network",
        version: "1",
        chain_id: provider.get_chain_id().await?,
        verifying_contract: contract_addr,
    };
    let bid = Bid {
        taskId: task_id,
        bidAmount: U256::from(4),
        bidder: user_addr,
    };
    let signature = signer.sign_typed_data(&bid, &domain).await?;
    let sig_hex = signature.to_string();

    let msg = json!({
        "bid": bid,
        "signature": sig_hex,
    });

    tokio::spawn(async move {
        println!("Bid:\n{}", serde_json::to_string_pretty(&msg).unwrap());
        print!("Send bid? (y/n): ");
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        let mut reader = io::BufReader::new(io::stdin());
        if reader.read_line(&mut input).await.is_ok() {
            if input.trim().eq_ignore_ascii_case("y") {
                if command_sender.send(Command::Publish(msg.to_string())).is_err() {
                    println!("Error sending publish command to swarm loop.");
                }
            } else {
                println!("Bid not sent.");
            }
        }
    });

    Ok(())
}

pub async fn handle_blockchain_event(
    log: Log,
    user_addr: Address,
    provider: &(impl Provider + Send + Sync),
    signer: &PrivateKeySigner,
    contract_addr: Address,
    command_sender: mpsc::UnboundedSender<Command>,
) -> Result<(), Box<dyn Error>> {
    match log.topic0() {
        Some(&ZINKNET::StakeDeposited::SIGNATURE_HASH) => {
            let ZINKNET::StakeDeposited { user, amount } = log.log_decode()?.inner.data;
            println!("StakeDeposited: user = {:?}, amount = {:?}", user, amount);
        }
        Some(&ZINKNET::StakeWithdrawn::SIGNATURE_HASH) => {
            let ZINKNET::StakeWithdrawn { user, amount } = log.log_decode()?.inner.data;
            println!("StakeWithdrawn: user = {:?}, amount = {:?}", user, amount);
        }
        Some(&ZINKNET::TaskCreated::SIGNATURE_HASH) => {
            let ZINKNET::TaskCreated { taskId, requestor, maxPayment } = log.log_decode()?.inner.data;
            println!("TaskCreated: taskId = {:?}, requestor = {:?}, maxPayment = {:?}", taskId, requestor, maxPayment);

            handle_task_created(taskId, user_addr, signer, provider, contract_addr, command_sender).await?;
        },
        _ => ()
    }
    Ok(())
}
