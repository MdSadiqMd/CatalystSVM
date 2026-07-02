//! SP1 guest program for proving CatalystSVM batch execution correctness
#![no_main]
sp1_zkvm::entrypoint!(main);

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTransaction {
    pub tx_id: String,
    pub sender: String,
    pub instruction_data: Vec<u8>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicInputs {
    pub batch_id: String,
    pub state_root_pre: String,
    pub state_root_post: String,
    pub trace_hash: String,
    pub total_compute: u64,
    pub tx_count: usize,
}

fn hash_bytes(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

fn hash_str(s: &str) -> String {
    hash_bytes(s.as_bytes())
}

fn merkle_root(leaves: &[String]) -> String {
    if leaves.is_empty() {
        return hash_bytes(b"empty");
    }
    if leaves.len() == 1 {
        return leaves[0].clone();
    }

    let mut current = leaves.to_vec();
    while current.len() > 1 {
        let mut next = Vec::new();
        for chunk in current.chunks(2) {
            let combined = if chunk.len() == 2 {
                format!("{}{}", chunk[0], chunk[1])
            } else {
                format!("{}{}", chunk[0], chunk[0])
            };
            next.push(hash_str(&combined));
        }
        current = next;
    }
    current[0].clone()
}

fn compute_state_root(state: &BTreeMap<String, u64>) -> String {
    if state.is_empty() {
        return hash_str("empty_state");
    }
    let leaves: Vec<String> = state
        .iter()
        .map(|(k, v)| hash_str(&format!("{}:{}", k, v)))
        .collect();
    merkle_root(&leaves)
}

fn hash_trace_entry(
    tx_id: &str,
    success: bool,
    compute_used: u64,
    accounts_read: &[String],
    accounts_written: &[String],
) -> String {
    let reads = accounts_read.join(",");
    let writes = accounts_written.join(",");
    let data = format!(
        "tx:{}:success:{}:cu:{}:reads:{}:writes:{}",
        tx_id, success, compute_used, reads, writes
    );
    hash_str(&data)
}

fn execute_instruction(
    state: &mut BTreeMap<String, u64>,
    sender: &str,
    instruction_data: &[u8],
) -> (Vec<String>, Vec<String>, u64, bool, Option<String>) {
    if instruction_data.is_empty() {
        return (vec![], vec![], 0, false, Some("empty instruction".into()));
    }

    let opcode = instruction_data[0];
    let data = &instruction_data[1..];

    match opcode {
        0 => {
            // Transfer
            if data.len() < 16 {
                return (vec![], vec![], 0, false, Some("invalid transfer data".into()));
            }
            let amount = u64::from_le_bytes(data[0..8].try_into().unwrap());
            let target_idx = u64::from_le_bytes(data[8..16].try_into().unwrap());
            let target = format!("account_{}", target_idx);

            let sender_balance = *state.get(sender).unwrap_or(&0);
            if sender_balance < amount {
                return (
                    vec![sender.to_string()],
                    vec![],
                    5000,
                    false,
                    Some("insufficient balance".into()),
                );
            }

            *state.entry(sender.to_string()).or_insert(0) -= amount;
            *state.entry(target.clone()).or_insert(0) += amount;

            (
                vec![sender.to_string(), target.clone()],
                vec![sender.to_string(), target],
                5000,
                true,
                None,
            )
        }
        1 => {
            // SetData
            if data.len() < 16 {
                return (vec![], vec![], 0, false, Some("invalid setdata".into()));
            }
            let key_idx = u64::from_le_bytes(data[0..8].try_into().unwrap());
            let value = u64::from_le_bytes(data[8..16].try_into().unwrap());
            let key = format!("data_{}", key_idx);

            state.insert(key.clone(), value);
            (vec![], vec![key], 3000, true, None)
        }
        2 => {
            // Increment
            if data.len() < 16 {
                return (vec![], vec![], 0, false, Some("invalid increment".into()));
            }
            let key_idx = u64::from_le_bytes(data[0..8].try_into().unwrap());
            let delta = u64::from_le_bytes(data[8..16].try_into().unwrap());
            let key = format!("data_{}", key_idx);

            let new_value = state.get(&key).unwrap_or(&0).saturating_add(delta);
            state.insert(key.clone(), new_value);
            (vec![key.clone()], vec![key], 2000, true, None)
        }
        3 => {
            // Noop
            (vec![], vec![], 1000, true, None)
        }
        _ => {
            // Invalid opcode
            (vec![], vec![], 500, false, Some("invalid opcode".into()))
        }
    }
}

fn execute_batch(
    initial_state: &BTreeMap<String, u64>,
    transactions: &[SerializableTransaction],
) -> (String, String, u64, Vec<String>) {
    let mut state = initial_state.clone();
    let state_root_pre = compute_state_root(&state);
    let mut total_compute = 0u64;
    let mut entry_hashes = Vec::new();

    for tx in transactions {
        let (reads, writes, compute, success, _error) =
            execute_instruction(&mut state, &tx.sender, &tx.instruction_data);
        total_compute += compute;

        let entry_hash = hash_trace_entry(&tx.tx_id, success, compute, &reads, &writes);
        entry_hashes.push(entry_hash);
    }

    let state_root_post = compute_state_root(&state);
    let _trace_hash = merkle_root(&entry_hashes);

    (state_root_pre, state_root_post, total_compute, entry_hashes)
}

pub fn main() {
    let trace: ExecutionTraceInput = sp1_zkvm::io::read();
    let transactions: Vec<SerializableTransaction> = sp1_zkvm::io::read();
    let initial_state: BTreeMap<String, u64> = sp1_zkvm::io::read();

    let (recomputed_pre, recomputed_post, recomputed_compute, _entry_hashes) =
        execute_batch(&initial_state, &transactions);

    let recomputed_trace_hash = merkle_root(&_entry_hashes);

    assert_eq!(
        recomputed_pre, trace.state_root_pre,
        "Pre-state root mismatch"
    );
    assert_eq!(
        recomputed_post, trace.state_root_post,
        "Post-state root mismatch"
    );
    assert_eq!(
        recomputed_trace_hash, trace.trace_hash,
        "Trace hash mismatch"
    );
    assert_eq!(
        recomputed_compute, trace.total_compute,
        "Total compute mismatch"
    );
    assert_eq!(
        transactions.len(),
        trace.tx_count,
        "Transaction count mismatch"
    );

    let public_inputs = PublicInputs {
        batch_id: trace.batch_id,
        state_root_pre: recomputed_pre,
        state_root_post: recomputed_post,
        trace_hash: recomputed_trace_hash,
        total_compute: recomputed_compute,
        tx_count: transactions.len(),
    };

    sp1_zkvm::io::commit(&public_inputs);
}
