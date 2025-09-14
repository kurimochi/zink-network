use alloy::{
    eips::BlockNumberOrTag,
    primitives::{Address, U256},
    providers::Provider,
    pubsub::SubscriptionStream,
    rpc::types::{Filter, Log},
    signers::local::PrivateKeySigner,
};
use std::{error::Error, time::Duration};
use tokio::time::sleep;

alloy::sol!(
    #[sol(rpc)]
    ZinKNetContract,
    "../../out/zinknet.sol/ZinKNetContract.json"
);

pub struct ZinKNet<P> {
    provider: P,
    contract: ZinKNetContract::ZinKNetContractInstance<P>,
}

impl<P: Provider + Clone> ZinKNet<P> {
    pub fn new(provider: P, address: Address) -> Self {
        let contract = ZinKNetContract::new(address, provider.clone());
        Self { provider, contract }
    }

    pub async fn create_task(
        &self,
        signer: &PrivateKeySigner,
        reward: U256,
    ) -> Result<U256, Box<dyn Error>> {
        println!("Creating task on blockchain...");

        let verification_gas = self.contract.verificationGas().call().await?;
        let receipt = self
            .contract
            .createTask(reward)
            .value(reward + verification_gas)
            .from(signer.address())
            .send()
            .await?
            .get_receipt()
            .await?;

        let taskcreated_log = receipt
            .decoded_log::<ZinKNetContract::TaskCreated>()
            .ok_or("Failed to find or decode TaskCreated log")?;

        let task_id = taskcreated_log.taskId;
        println!("Created task with ID: {}", task_id);
        Ok(task_id)
    }
}

impl<P: Provider + Send + Sync + 'static + Clone> ZinKNet<P> {
    // Use when RPC supports event subscription (e.g. WebSocket).
    pub async fn setup_ethlistener(&self) -> Result<SubscriptionStream<Log>, Box<dyn Error>> {
        let filter = Filter::new()
            .address(self.contract.address().clone())
            .from_block(BlockNumberOrTag::Latest);

        let sub = self.provider.subscribe_logs(&filter).await?;
        Ok(sub.into_stream())
    }

    // Use if RPC does not support event subscription (e.g. HTTP, Anvil).
    pub async fn setup_ethlistener_polling(
        &self,
    ) -> Result<tokio_stream::wrappers::UnboundedReceiverStream<Log>, Box<dyn Error>> {
        let base_filter = Filter::new().address(self.contract.address().clone());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let provider = self.provider.clone();

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
}
