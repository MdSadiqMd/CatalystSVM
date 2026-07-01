//! Deterministic execution engine for batch transaction replay
use catalyst_common::{Batch, Clock, ExecError, ExecutionTrace, Executor};
use catalyst_trace_builder::{TraceBuilder, compute_state_root};
use std::collections::BTreeMap;

/// Instruction opcodes for the restricted instruction set
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Transfer = 0,
    SetData = 1,
    Increment = 2,
    Noop = 3,
    Invalid = 255,
}

impl From<u8> for Opcode {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Transfer,
            1 => Self::SetData,
            2 => Self::Increment,
            3 => Self::Noop,
            _ => Self::Invalid,
        }
    }
}

/// Deterministic executor using BTreeMap for ordered state
#[derive(Debug)]
pub struct DeterministicExecutor {
    max_cu_per_tx: u64,
    fail_on_invalid: bool,
}

impl DeterministicExecutor {
    pub fn new(max_cu_per_tx: u64) -> Self {
        Self {
            max_cu_per_tx,
            fail_on_invalid: true,
        }
    }

    pub fn with_fail_on_invalid(mut self, fail: bool) -> Self {
        self.fail_on_invalid = fail;
        self
    }

    fn execute_instruction(
        &self,
        state: &mut BTreeMap<String, u64>,
        sender: &str,
        instruction_data: &[u8],
    ) -> Result<(Vec<String>, Vec<String>, u64), ExecError> {
        if instruction_data.is_empty() {
            return Err(ExecError::InvalidOpcode(255));
        }

        let opcode = Opcode::from(instruction_data[0]);
        let data = &instruction_data[1..];

        match opcode {
            Opcode::Transfer => self.execute_transfer(state, sender, data),
            Opcode::SetData => self.execute_set_data(state, data),
            Opcode::Increment => self.execute_increment(state, data),
            Opcode::Noop => Ok((vec![], vec![], 1000)),
            Opcode::Invalid => {
                if self.fail_on_invalid {
                    Err(ExecError::InvalidOpcode(instruction_data[0]))
                } else {
                    Ok((vec![], vec![], 500))
                }
            }
        }
    }

    fn execute_transfer(
        &self,
        state: &mut BTreeMap<String, u64>,
        sender: &str,
        data: &[u8],
    ) -> Result<(Vec<String>, Vec<String>, u64), ExecError> {
        if data.len() < 16 {
            return Err(ExecError::InvalidOpcode(0));
        }

        let amount = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let target_idx = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let target = format!("account_{}", target_idx);

        let sender_balance = *state.get(sender).unwrap_or(&0);
        if sender_balance < amount {
            return Err(ExecError::InsufficientBalance {
                needed: amount,
                available: sender_balance,
            });
        }

        *state.entry(sender.to_string()).or_insert(0) -= amount;
        *state.entry(target.clone()).or_insert(0) += amount;

        let reads = vec![sender.to_string(), target.clone()];
        let writes = vec![sender.to_string(), target];

        Ok((reads, writes, 5000))
    }

    fn execute_set_data(
        &self,
        state: &mut BTreeMap<String, u64>,
        data: &[u8],
    ) -> Result<(Vec<String>, Vec<String>, u64), ExecError> {
        if data.len() < 16 {
            return Err(ExecError::InvalidOpcode(1));
        }

        let key_idx = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let value = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let key = format!("data_{}", key_idx);

        state.insert(key.clone(), value);

        let reads = vec![];
        let writes = vec![key];

        Ok((reads, writes, 3000))
    }

    fn execute_increment(
        &self,
        state: &mut BTreeMap<String, u64>,
        data: &[u8],
    ) -> Result<(Vec<String>, Vec<String>, u64), ExecError> {
        if data.len() < 16 {
            return Err(ExecError::InvalidOpcode(2));
        }

        let key_idx = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let delta = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let key = format!("data_{}", key_idx);

        let new_value = state.get(&key).unwrap_or(&0).saturating_add(delta);
        state.insert(key.clone(), new_value);

        let reads = vec![key.clone()];
        let writes = vec![key];

        Ok((reads, writes, 2000))
    }
}

impl Executor for DeterministicExecutor {
    fn execute(&self, batch: &Batch, clock: &dyn Clock) -> Result<ExecutionTrace, ExecError> {
        let _start_time = clock.now_ms();
        let mut state: BTreeMap<String, u64> = BTreeMap::new();

        // Initialize sender balances
        for tx_id in &batch.tx_ids {
            state
                .entry(format!("sender_{}", tx_id.as_str()))
                .or_insert(1_000_000);
        }

        let mut builder = TraceBuilder::new(batch.batch_id.clone(), &state);

        // Note: In a real implementation, we'd have access to the transactions
        // For now, we'll simulate based on tx_ids
        for (idx, tx_id) in batch.tx_ids.iter().enumerate() {
            let pre_state_hash = compute_state_root(&state);

            // Simulate a simple instruction based on tx_id hash
            let instruction_data = vec![3u8]; // Noop for now
            let sender = format!("sender_{}", tx_id.as_str());

            let result = self.execute_instruction(&mut state, &sender, &instruction_data);

            let post_state_hash = compute_state_root(&state);

            match result {
                Ok((reads, writes, compute_used)) => {
                    let compute_used = compute_used.min(self.max_cu_per_tx);
                    builder.add_entry(
                        idx,
                        tx_id.clone(),
                        pre_state_hash,
                        "execute".into(),
                        reads,
                        writes,
                        post_state_hash,
                        compute_used,
                        true,
                        None,
                    );
                }
                Err(e) => {
                    builder.add_entry(
                        idx,
                        tx_id.clone(),
                        pre_state_hash,
                        "execute".into(),
                        vec![],
                        vec![],
                        post_state_hash,
                        0,
                        false,
                        Some(e.to_string()),
                    );
                }
            }
        }

        let trace = builder.build(&state);

        Ok(trace)
    }
}

/// Extended executor that works with full Transaction data
#[derive(Debug)]
pub struct FullExecutor {
    inner: DeterministicExecutor,
}

impl FullExecutor {
    pub fn new(max_cu_per_tx: u64) -> Self {
        Self {
            inner: DeterministicExecutor::new(max_cu_per_tx),
        }
    }

    pub fn execute_with_transactions(
        &self,
        batch: &Batch,
        transactions: &[catalyst_common::Transaction],
        clock: &dyn Clock,
    ) -> Result<ExecutionTrace, ExecError> {
        let _start_time = clock.now_ms();
        let mut state: BTreeMap<String, u64> = BTreeMap::new();

        // Initialize sender balances
        for tx in transactions {
            state.entry(tx.sender.clone()).or_insert(1_000_000);
        }

        let mut builder = TraceBuilder::new(batch.batch_id.clone(), &state);

        for (idx, tx) in transactions.iter().enumerate() {
            let pre_state_hash = compute_state_root(&state);

            let result =
                self.inner
                    .execute_instruction(&mut state, &tx.sender, &tx.instruction_data);

            let post_state_hash = compute_state_root(&state);

            match result {
                Ok((reads, writes, compute_used)) => {
                    let compute_used = compute_used.min(self.inner.max_cu_per_tx);
                    builder.add_entry(
                        idx,
                        tx.tx_id.clone(),
                        pre_state_hash,
                        format!("opcode_{}", tx.instruction_data.first().unwrap_or(&255)),
                        reads,
                        writes,
                        post_state_hash,
                        compute_used,
                        true,
                        None,
                    );
                }
                Err(e) => {
                    builder.add_entry(
                        idx,
                        tx.tx_id.clone(),
                        pre_state_hash,
                        "failed".into(),
                        vec![],
                        vec![],
                        post_state_hash,
                        0,
                        false,
                        Some(e.to_string()),
                    );
                }
            }
        }

        let trace = builder.build(&state);

        Ok(trace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalyst_common::{Transaction, TransactionId, TransactionPriority, VirtualClock};

    fn make_tx(id: &str, opcode: u8, data: &[u8]) -> Transaction {
        let mut instruction_data = vec![opcode];
        instruction_data.extend_from_slice(data);

        let mut tx = Transaction::new(TransactionId::new(id), "alice", "test_program");
        tx.instruction_data = instruction_data;
        tx.estimated_cu = 100_000;
        tx.priority = TransactionPriority::Normal;
        tx.arrival_ts = 1000;
        tx
    }

    fn make_batch(tx_ids: Vec<TransactionId>) -> Batch {
        let mut batch = Batch::new("batch_1", 1000);
        batch.tx_ids = tx_ids;
        batch
    }

    #[test]
    fn test_execute_basic() {
        let executor = DeterministicExecutor::new(200_000);
        let clock = VirtualClock::new(0);

        let batch = make_batch(vec![TransactionId::new("tx_1")]);

        let trace = executor.execute(&batch, &clock).unwrap();
        assert_eq!(trace.entries.len(), 1);
        assert!(trace.entries[0].success);
    }

    #[test]
    fn test_full_executor_noop() {
        let executor = FullExecutor::new(200_000);
        let clock = VirtualClock::new(0);

        let tx = make_tx("tx_1", 3, &[]); // Noop
        let batch = make_batch(vec![tx.tx_id.clone()]);

        let trace = executor
            .execute_with_transactions(&batch, &[tx], &clock)
            .unwrap();
        assert_eq!(trace.entries.len(), 1);
        assert!(trace.entries[0].success);
    }

    #[test]
    fn test_full_executor_set_data() {
        let executor = FullExecutor::new(200_000);
        let clock = VirtualClock::new(0);

        let key: u64 = 5;
        let value: u64 = 12345;
        let mut data = Vec::new();
        data.extend_from_slice(&key.to_le_bytes());
        data.extend_from_slice(&value.to_le_bytes());

        let tx = make_tx("tx_1", 1, &data); // SetData
        let batch = make_batch(vec![tx.tx_id.clone()]);

        let trace = executor
            .execute_with_transactions(&batch, &[tx], &clock)
            .unwrap();
        assert!(trace.entries[0].success);
        assert!(
            trace.entries[0]
                .accounts_written
                .contains(&"data_5".to_string())
        );
    }

    #[test]
    fn test_deterministic_execution() {
        let executor = FullExecutor::new(200_000);
        let clock1 = VirtualClock::new(0);
        let clock2 = VirtualClock::new(0);

        let key: u64 = 1;
        let value: u64 = 999;
        let mut data = Vec::new();
        data.extend_from_slice(&key.to_le_bytes());
        data.extend_from_slice(&value.to_le_bytes());

        let tx1 = make_tx("tx_1", 1, &data);
        let tx2 = make_tx("tx_2", 2, &data);
        let batch1 = make_batch(vec![tx1.tx_id.clone(), tx2.tx_id.clone()]);
        let batch2 = make_batch(vec![tx1.tx_id.clone(), tx2.tx_id.clone()]);

        let trace1 = executor
            .execute_with_transactions(&batch1, &[tx1.clone(), tx2.clone()], &clock1)
            .unwrap();
        let trace2 = executor
            .execute_with_transactions(&batch2, &[tx1, tx2], &clock2)
            .unwrap();

        assert_eq!(trace1.trace_hash, trace2.trace_hash);
        assert_eq!(trace1.state_root_post, trace2.state_root_post);
    }
}
