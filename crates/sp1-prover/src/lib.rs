//! SP1 prover for CatalystSVM batch execution proofs
use catalyst_common::{
    Clock, ExecutionTrace, Proof, ProofCommitment, ProveError, Prover as CatalystProver,
    Transaction, hash_bytes,
};
use serde::{Deserialize, Serialize};
use sp1_sdk::Elf;
use sp1_sdk::blocking::{CpuProver, ProveRequest, Prover, SP1Stdin};
use std::collections::BTreeMap;
use std::time::Instant;

const SP1_ELF: &[u8] = include_bytes!("../../sp1-program/elf/riscv32im-succinct-zkvm-elf");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTransaction {
    pub tx_id: String,
    pub sender: String,
    pub instruction_data: Vec<u8>,
}

impl From<&Transaction> for SerializableTransaction {
    fn from(tx: &Transaction) -> Self {
        Self {
            tx_id: tx.tx_id.as_str().to_string(),
            sender: tx.sender.clone(),
            instruction_data: tx.instruction_data.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTraceInput {
    pub batch_id: String,
    pub state_root_pre: String,
    pub state_root_post: String,
    pub trace_hash: String,
    pub total_compute: u64,
    pub tx_count: usize,
}

impl From<&ExecutionTrace> for ExecutionTraceInput {
    fn from(trace: &ExecutionTrace) -> Self {
        Self {
            batch_id: trace.batch_id.clone(),
            state_root_pre: trace.state_root_pre.clone(),
            state_root_post: trace.state_root_post.clone(),
            trace_hash: trace.trace_hash.clone(),
            total_compute: trace.total_compute,
            tx_count: trace.entries.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicInputs {
    pub batch_id: String,
    pub state_root_pre: String,
    pub state_root_post: String,
    pub trace_hash: String,
    pub total_compute: u64,
    pub tx_count: usize,
}

pub struct Sp1Prover {
    prover: CpuProver,
}

impl std::fmt::Debug for Sp1Prover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Prover").finish()
    }
}

impl Default for Sp1Prover {
    fn default() -> Self {
        Self::new()
    }
}

impl Sp1Prover {
    pub fn new() -> Self {
        use sp1_sdk::blocking::ProverClient;
        Self {
            prover: ProverClient::builder().cpu().build(),
        }
    }

    /// Fast execution (no proof) for debugging guest logic
    pub fn execute_only(
        &self,
        trace: &ExecutionTrace,
        transactions: &[Transaction],
        initial_state: &BTreeMap<String, u64>,
    ) -> Result<(), ProveError> {
        let elf = Elf::from(SP1_ELF);

        let trace_input = ExecutionTraceInput::from(trace);
        let serializable_txs: Vec<SerializableTransaction> = transactions
            .iter()
            .map(SerializableTransaction::from)
            .collect();

        let mut stdin = SP1Stdin::new();
        stdin.write(&trace_input);
        stdin.write(&serializable_txs);
        stdin.write(initial_state);

        self.prover
            .execute(elf, stdin)
            .run()
            .map_err(|e| ProveError::Internal(e.to_string()))?;

        Ok(())
    }

    pub fn prove_with_transactions(
        &self,
        trace: &ExecutionTrace,
        transactions: &[Transaction],
        initial_state: &BTreeMap<String, u64>,
        clock: &dyn Clock,
    ) -> Result<Proof, ProveError> {
        if trace.entries.is_empty() {
            return Err(ProveError::InvalidTrace("empty trace".into()));
        }

        let start = Instant::now();

        let elf = Elf::from(SP1_ELF);
        let pk = self
            .prover
            .setup(elf)
            .map_err(|e| ProveError::Internal(e.to_string()))?;

        let trace_input = ExecutionTraceInput::from(trace);
        let serializable_txs: Vec<SerializableTransaction> = transactions
            .iter()
            .map(SerializableTransaction::from)
            .collect();

        let mut stdin = SP1Stdin::new();
        stdin.write(&trace_input);
        stdin.write(&serializable_txs);
        stdin.write(initial_state);

        let sp1_proof = self
            .prover
            .prove(&pk, stdin)
            .compressed()
            .run()
            .map_err(|e| ProveError::Internal(e.to_string()))?;

        let proving_time_ms = start.elapsed().as_millis() as u64;

        let proof_bytes =
            bincode::serialize(&sp1_proof).map_err(|e| ProveError::Internal(e.to_string()))?;

        let proof_hash = hash_bytes(&proof_bytes);

        let commitment = ProofCommitment {
            batch_hash: catalyst_common::hash_str(&trace.batch_id),
            trace_hash: trace.trace_hash.clone(),
            state_root_pre: trace.state_root_pre.clone(),
            state_root_post: trace.state_root_post.clone(),
            total_compute: trace.total_compute,
            tx_count: trace.entries.len(),
        };

        clock.sleep_ms(proving_time_ms);

        Ok(Proof {
            batch_id: trace.batch_id.clone(),
            proof_data: proof_bytes.clone(),
            proof_hash,
            commitment,
            generated_at: clock.now_ms(),
            proving_time_ms,
            proof_size_bytes: proof_bytes.len(),
        })
    }
}

impl CatalystProver for Sp1Prover {
    fn prove(&self, trace: &ExecutionTrace, clock: &dyn Clock) -> Result<Proof, ProveError> {
        // Reconstruct the same initial state and transactions that DeterministicExecutor used
        let mut initial_state: BTreeMap<String, u64> = BTreeMap::new();

        // DeterministicExecutor initializes sender balances based on tx_ids
        for entry in &trace.entries {
            let sender = format!("sender_{}", entry.tx_id.as_str());
            initial_state.entry(sender).or_insert(1_000_000);
        }

        // DeterministicExecutor uses Noop (opcode 3) for all transactions
        let transactions: Vec<Transaction> = trace
            .entries
            .iter()
            .map(|e| {
                let sender = format!("sender_{}", e.tx_id.as_str());
                let mut tx = Transaction::new(e.tx_id.clone(), sender, "execute");
                tx.instruction_data = vec![3u8]; // Noop
                tx
            })
            .collect();

        self.prove_with_transactions(trace, &transactions, &initial_state, clock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalyst_common::{TraceEntry, TransactionId, VirtualClock};

    fn create_test_trace(tx_count: usize) -> ExecutionTrace {
        let entries: Vec<TraceEntry> = (0..tx_count)
            .map(|i| TraceEntry {
                tx_idx: i,
                tx_id: TransactionId::new(format!("tx_{}", i)),
                pre_state_hash: format!("pre_{}", i),
                instruction: "noop".into(),
                accounts_read: vec![],
                accounts_written: vec![],
                post_state_hash: format!("post_{}", i),
                compute_used: 1000,
                success: true,
                error: None,
            })
            .collect();

        ExecutionTrace {
            batch_id: "test_batch".into(),
            entries,
            state_root_pre: "pre_state_root".into(),
            state_root_post: "post_state_root".into(),
            trace_hash: "trace_hash".into(),
            total_compute: tx_count as u64 * 1000,
            success_count: tx_count,
            failure_count: 0,
        }
    }

    #[test]
    fn test_prover_creation() {
        let _prover = Sp1Prover::new();
    }

    #[test]
    fn test_guest_execute_matches_executor() {
        use catalyst_common::{Batch, Executor};
        use catalyst_execution_engine::DeterministicExecutor;

        // Use 5 txs (odd) to exercise the odd-leaf merkle path
        let mut batch = Batch::new("test_batch", 0);
        batch.tx_ids = (1..=5)
            .map(|i| TransactionId::new(format!("tx_{:08x}", i)))
            .collect();

        let executor = DeterministicExecutor::new(200_000);
        let clock = VirtualClock::new(0);
        let trace = executor.execute(&batch, &clock).unwrap();

        let mut initial_state: BTreeMap<String, u64> = BTreeMap::new();
        for entry in &trace.entries {
            initial_state
                .entry(format!("sender_{}", entry.tx_id.as_str()))
                .or_insert(1_000_000);
        }
        let transactions: Vec<Transaction> = trace
            .entries
            .iter()
            .map(|e| {
                let sender = format!("sender_{}", e.tx_id.as_str());
                let mut tx = Transaction::new(e.tx_id.clone(), sender, "execute");
                tx.instruction_data = vec![3u8];
                tx
            })
            .collect();

        // Runs the guest in SP1's emulator; all internal assertions must pass
        let prover = Sp1Prover::new();
        let result = prover.execute_only(&trace, &transactions, &initial_state);
        assert!(result.is_ok(), "Guest execution failed: {:?}", result);
    }

    #[test]
    fn test_state_reconstruction_matches_executor() {
        use catalyst_common::{Batch, Executor};
        use catalyst_execution_engine::DeterministicExecutor;
        use catalyst_trace_builder::compute_state_root;

        let mut batch = Batch::new("test_batch", 0);
        // Use exact same format as validator: tx_{:08x} with 5 txs
        batch.tx_ids = vec![
            TransactionId::new(format!("tx_{:08x}", 1)),
            TransactionId::new(format!("tx_{:08x}", 2)),
            TransactionId::new(format!("tx_{:08x}", 3)),
            TransactionId::new(format!("tx_{:08x}", 4)),
            TransactionId::new(format!("tx_{:08x}", 5)),
        ];

        let executor = DeterministicExecutor::new(200_000);
        let clock = VirtualClock::new(0);
        let trace = executor.execute(&batch, &clock).unwrap();

        // Reconstruct what prover does
        let mut initial_state: BTreeMap<String, u64> = BTreeMap::new();
        for entry in &trace.entries {
            let sender = format!("sender_{}", entry.tx_id.as_str());
            initial_state.entry(sender).or_insert(1_000_000);
        }

        let reconstructed_pre = compute_state_root(&initial_state);

        eprintln!("Trace state_root_pre:         {}", trace.state_root_pre);
        eprintln!("Reconstructed state_root_pre: {}", reconstructed_pre);
        eprintln!("State keys:");
        for (k, v) in &initial_state {
            eprintln!("  {}: {}", k, v);
        }

        assert_eq!(
            trace.state_root_pre, reconstructed_pre,
            "State root mismatch between executor and prover reconstruction"
        );
    }

    #[test]
    fn test_serializable_transaction() {
        let tx = Transaction::new(TransactionId::new("tx_1"), "alice", "program_1");
        let serializable = SerializableTransaction::from(&tx);
        assert_eq!(serializable.tx_id, "tx_1");
        assert_eq!(serializable.sender, "alice");
    }

    #[test]
    #[ignore]
    fn test_sp1_prove() {
        let trace = create_test_trace(2);
        let clock = VirtualClock::new(0);
        let prover = Sp1Prover::new();

        let result = prover.prove(&trace, &clock);
        assert!(result.is_ok());

        let proof = result.unwrap();
        assert!(!proof.proof_data.is_empty());
        assert!(proof.proving_time_ms > 0);
    }
}
