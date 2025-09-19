use alloy::{primitives::U256, signers::Signature};
use serde::{Deserialize, Serialize};
use sp1_sdk::SP1Stdin;

#[derive(Deserialize, Serialize)]
pub struct ElfPayload {
    pub competition_id: U256,
    pub elf: Vec<u8>,
    pub stdin: SP1Stdin,
    pub signature: Signature,
}
