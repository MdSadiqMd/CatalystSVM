//! SP1 verifier for CatalystSVM batch execution proofs
use catalyst_common::{
    Clock, ExecutionTrace, Proof, VerificationResult, Verifier as CatalystVerifier, VerifyError,
};
use serde::{Deserialize, Serialize};
use sp1_sdk::blocking::{CpuProver, Prover, ProverClient};
use sp1_sdk::{Elf, ProvingKey, SP1ProofWithPublicValues};
use std::time::Instant;

const SP1_ELF: &[u8] = include_bytes!("../../sp1-program/elf/riscv32im-succinct-zkvm-elf");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicInputs {
    pub batch_id: String,
    pub state_root_pre: String,
    pub state_root_post: String,
    pub trace_hash: String,
    pub total_compute: u64,
    pub tx_count: usize,
}

pub struct Sp1Verifier {
    prover: CpuProver,
}

impl std::fmt::Debug for Sp1Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Verifier").finish()
    }
}

impl Default for Sp1Verifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Sp1Verifier {
    pub fn new() -> Self {
        Self {
            prover: ProverClient::builder().cpu().build(),
        }
    }
}

impl CatalystVerifier for Sp1Verifier {
    fn verify(
        &self,
        proof: &Proof,
        trace: &ExecutionTrace,
        clock: &dyn Clock,
    ) -> Result<VerificationResult, VerifyError> {
        let start = Instant::now();

        let mut sp1_proof: SP1ProofWithPublicValues = bincode::deserialize(&proof.proof_data)
            .map_err(|e| {
                VerifyError::VerificationFailed(format!("failed to deserialize proof: {}", e))
            })?;

        let elf = Elf::from(SP1_ELF);
        let pk = self.prover.setup(elf).map_err(|e| {
            VerifyError::VerificationFailed(format!("failed to setup verifier: {}", e))
        })?;

        let vk = pk.verifying_key();

        let verification_result = self.prover.verify(&sp1_proof, &vk, None);

        let verification_time_ms = start.elapsed().as_millis() as u64;
        clock.sleep_ms(verification_time_ms);

        match verification_result {
            Ok(()) => {
                let public_inputs: PublicInputs = sp1_proof.public_values.read();

                if public_inputs.trace_hash != trace.trace_hash {
                    return Ok(VerificationResult {
                        batch_id: proof.batch_id.clone(),
                        is_valid: false,
                        verification_time_ms,
                        error: Some(format!(
                            "trace hash mismatch: expected {}, got {}",
                            trace.trace_hash, public_inputs.trace_hash
                        )),
                    });
                }

                if public_inputs.state_root_pre != trace.state_root_pre {
                    return Ok(VerificationResult {
                        batch_id: proof.batch_id.clone(),
                        is_valid: false,
                        verification_time_ms,
                        error: Some("pre-state root mismatch".into()),
                    });
                }

                if public_inputs.state_root_post != trace.state_root_post {
                    return Ok(VerificationResult {
                        batch_id: proof.batch_id.clone(),
                        is_valid: false,
                        verification_time_ms,
                        error: Some("post-state root mismatch".into()),
                    });
                }

                if public_inputs.total_compute != trace.total_compute {
                    return Ok(VerificationResult {
                        batch_id: proof.batch_id.clone(),
                        is_valid: false,
                        verification_time_ms,
                        error: Some("total compute mismatch".into()),
                    });
                }

                Ok(VerificationResult {
                    batch_id: proof.batch_id.clone(),
                    is_valid: true,
                    verification_time_ms,
                    error: None,
                })
            }
            Err(e) => Ok(VerificationResult {
                batch_id: proof.batch_id.clone(),
                is_valid: false,
                verification_time_ms,
                error: Some(format!("SP1 verification failed: {}", e)),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_creation() {
        let _verifier = Sp1Verifier::new();
    }
}
