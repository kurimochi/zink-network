use alloy::{
    eips::BlockNumberOrTag,
    primitives::{Address, U256},
    providers::Provider,
    rpc::types::{Filter, Log},
    signers::{Signer, local::PrivateKeySigner},
    sol,
    sol_types::{SolEvent, eip712_domain},
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{error::Error, io::Write, time::Duration};
use tokio::{
    io::{self, AsyncBufReadExt},
    sync::{Mutex, mpsc},
    time::sleep,
};

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

static STDIN_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub enum Command {
    Publish(String),
}

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

async fn handle_task_created<P>(
    task_id: U256,
    max_payment: U256,
    user_addr: Address,
    signer: &PrivateKeySigner,
    provider: P,
    contract_addr: Address,
    command_sender: mpsc::UnboundedSender<Command>,
) -> Result<(), Box<dyn Error>>
where
    P: Provider + Send + Sync + Clone + 'static,
{
    let signer = signer.clone();
    let command_sender = command_sender.clone();

    tokio::spawn(async move {
        let _guard = STDIN_MUTEX.lock().await;

        let assigned_filter = Filter::new()
            .address(contract_addr)
            .event_signature(ZINKNET::ProverAssigned::SIGNATURE_HASH)
            .topic1(task_id);

        match provider.get_logs(&assigned_filter).await {
            Ok(logs) if !logs.is_empty() => {
                println!(
                    "\nTask {} has already been assigned. Skipping bid.",
                    task_id
                );
                return;
            }
            Err(e) => {
                println!("\nError checking task assignment: {:?}. Skipping bid.", e);
                return;
            }
            _ => {} // Not assigned, proceed.
        }

        let chain_id = match provider.get_chain_id().await {
            Ok(id) => id,
            Err(e) => {
                println!("\nFailed to get chain ID: {}. Bid cancelled.", e);
                return;
            }
        };
        let domain = eip712_domain! {
            name: "ZinK Network",
            version: "1",
            chain_id: chain_id,
            verifying_contract: contract_addr,
        };

        println!("\n========================================");
        println!("  New Task Available for Bidding");
        println!("----------------------------------------");
        println!("  Task ID: {}", task_id);
        println!("  Max Payment: {}", max_payment);
        println!("========================================");
        print!("Enter your bid amount, or press Enter to skip: ");
        std::io::stdout().flush().unwrap();

        let mut reader = io::BufReader::new(io::stdin());
        let mut input = String::new();
        if reader.read_line(&mut input).await.is_err() {
            println!("Failed to read input. Skipping bid.");
            return;
        }

        let trimmed_input = input.trim();
        if trimmed_input.is_empty() {
            println!("Skipping bid.");
            return;
        }

        let bid_amount = match trimmed_input.parse::<U256>() {
            Ok(amount) => {
                if amount < U256::from(1) || amount > max_payment {
                    println!(
                        "Bid amount must be between 1 and {}. Bid cancelled.",
                        max_payment
                    );
                    return;
                }
                amount
            }
            Err(_) => {
                println!("Invalid amount entered. Bid cancelled.");
                return;
            }
        };

        let bid = Bid {
            taskId: task_id,
            bidAmount: bid_amount,
            bidder: user_addr,
        };
        let signature = match signer.sign_typed_data(&bid, &domain).await {
            Ok(sig) => sig,
            Err(e) => {
                println!("Failed to sign bid: {}. Bid cancelled.", e);
                return;
            }
        };
        let sig_hex = signature.to_string();

        let msg = json!({
            "bid": bid,
            "signature": sig_hex,
        });

        println!("\n--- Bid Preview ---");
        println!("{}", serde_json::to_string_pretty(&msg).unwrap());
        println!("-------------------");
        print!("Send this bid? (y/n): ");
        std::io::stdout().flush().unwrap();

        input.clear();
        if reader.read_line(&mut input).await.is_ok() {
            if input.trim().eq_ignore_ascii_case("y") {
                if command_sender
                    .send(Command::Publish(msg.to_string()))
                    .is_err()
                {
                    println!("Error sending publish command to swarm loop.");
                } else {
                    println!("Bid sent successfully!");
                }
            } else {
                println!("Bid cancelled.");
            }
        } else {
            println!("Failed to read confirmation. Bid cancelled.");
        }
    });

    Ok(())
}

pub async fn handle_blockchain_event<P>(
    log: Log,
    user_addr: Address,
    provider: P,
    signer: &PrivateKeySigner,
    contract_addr: Address,
    command_sender: mpsc::UnboundedSender<Command>,
) -> Result<(), Box<dyn Error>>
where
    P: Provider + Send + Sync + Clone + 'static,
{
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
            let ZINKNET::TaskCreated {
                taskId,
                requestor,
                maxPayment,
            } = log.log_decode()?.inner.data;
            println!(
                "TaskCreated: taskId = {:?}, requestor = {:?}, maxPayment = {:?}",
                taskId, requestor, maxPayment
            );

            if requestor != user_addr {
                handle_task_created(
                    taskId,
                    maxPayment,
                    user_addr,
                    signer,
                    provider,
                    contract_addr,
                    command_sender,
                )
                .await?;
            }
        }
        Some(&ZINKNET::ProverAssigned::SIGNATURE_HASH) => {
            let ZINKNET::ProverAssigned {
                taskId,
                prover,
                finalPayment,
            } = log.log_decode()?.inner.data;
            println!(
                "ProverAssigned: taskId = {:?}, prover = {:?}, finalPayment = {:?}",
                taskId, prover, finalPayment
            );
        }
        _ => (),
    }
    Ok(())
}
