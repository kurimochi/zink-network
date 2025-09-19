use sp1_sdk::{ProverClient, SP1ProofWithPublicValues, SP1Stdin};
use std::error::Error;

pub fn sp1_proof(elf: &[u8], stdin: &SP1Stdin) -> Result<SP1ProofWithPublicValues, Box<dyn Error>> {
    let client = ProverClient::from_env();
    let (pk, vk) = client.setup(elf);
    let proof = client.prove(&pk, &stdin).groth16().run()?;
    client.verify(&proof, &vk)?;

    Ok(proof)
}
