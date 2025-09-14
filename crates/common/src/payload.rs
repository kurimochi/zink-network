use alloy::{primitives::U256, signers::Signature};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct ElfPayload {
    pub task_id: U256,
    pub elf: Vec<u8>,
    pub signature: Signature,
}
