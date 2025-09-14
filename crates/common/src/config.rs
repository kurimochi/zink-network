use alloy::primitives::Address;
use clap::Args;

#[derive(Args, Debug)]
pub struct CommonConfig {
    #[arg(long, env = "RPC_URL")]
    pub rpc_url: String,

    #[arg(long, value_name = "CONTRACT_ADDRESS", env = "CONTRACT_ADDRESS")]
    pub contract: Address,

    #[arg(long, env = "PRIVATE_KEY")]
    pub private_key: String,
}
