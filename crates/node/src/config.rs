use alloy::{primitives::Address, signers::local::PrivateKeySigner};
use std::env;

pub struct Config {
    pub rpc_url: String,
    pub contract_address: Address,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        dotenv::dotenv().ok();
        let rpc_url = env::var("RPC_URL").expect("RPC_URL must be set");
        let contract_address = env::var("CONTRACT_ADDRESS")
            .expect("CONTRACT_ADDRESS must be set")
            .parse()?;
        Ok(Self {
            rpc_url,
            contract_address,
        })
    }
}

pub fn get_signer_from_env(key_name: &str) -> Result<PrivateKeySigner, Box<dyn std::error::Error>> {
    let private_key = env::var(key_name)?;
    Ok(private_key.parse()?)
}
