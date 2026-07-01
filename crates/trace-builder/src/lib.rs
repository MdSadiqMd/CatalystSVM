//! Trace building for execution traces with Merkle commitments
use catalyst_common::{
    ExecutionTrace, TraceEntry, TransactionId, hash_str, hash_trace_entry, merkle_root,
};
use std::collections::BTreeMap;

/// Build execution traces with proper hash commitments
#[derive(Debug)]
pub struct TraceBuilder {
    batch_id: String,
    entries: Vec<TraceEntry>,
    pre_state_root: String,
    total_compute: u64,
    success_count: usize,
    failure_count: usize,
}

impl TraceBuilder {
    pub fn new(batch_id: String, initial_state: &BTreeMap<String, u64>) -> Self {
        Self {
            batch_id,
            entries: Vec::new(),
            pre_state_root: compute_state_root(initial_state),
            total_compute: 0,
            success_count: 0,
            failure_count: 0,
        }
    }

    pub fn add_entry(
        &mut self,
        tx_idx: usize,
        tx_id: TransactionId,
        pre_state_hash: String,
        instruction: String,
        accounts_read: Vec<String>,
        accounts_written: Vec<String>,
        post_state_hash: String,
        compute_used: u64,
        success: bool,
        error: Option<String>,
    ) {
        self.total_compute += compute_used;
        if success {
            self.success_count += 1;
        } else {
            self.failure_count += 1;
        }

        self.entries.push(TraceEntry {
            tx_idx,
            tx_id,
            pre_state_hash,
            instruction,
            accounts_read,
            accounts_written,
            post_state_hash,
            compute_used,
            success,
            error,
        });
    }

    pub fn build(self, final_state: &BTreeMap<String, u64>) -> ExecutionTrace {
        let post_state_root = compute_state_root(final_state);

        let entry_hashes: Vec<String> = self
            .entries
            .iter()
            .map(|e| {
                hash_trace_entry(
                    e.tx_id.as_str(),
                    e.success,
                    e.compute_used,
                    &e.accounts_read,
                    &e.accounts_written,
                )
            })
            .collect();

        let trace_hash = merkle_root(&entry_hashes);

        ExecutionTrace {
            batch_id: self.batch_id,
            entries: self.entries,
            state_root_pre: self.pre_state_root,
            state_root_post: post_state_root,
            trace_hash,
            total_compute: self.total_compute,
            success_count: self.success_count,
            failure_count: self.failure_count,
        }
    }
}

/// Compute deterministic state root from sorted state map
pub fn compute_state_root(state: &BTreeMap<String, u64>) -> String {
    if state.is_empty() {
        return hash_str("empty_state");
    }

    let leaves: Vec<String> = state
        .iter()
        .map(|(k, v)| hash_str(&format!("{}:{}", k, v)))
        .collect();

    merkle_root(&leaves)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_root_deterministic() {
        let mut state = BTreeMap::new();
        state.insert("alice".into(), 100);
        state.insert("bob".into(), 200);

        let r1 = compute_state_root(&state);
        let r2 = compute_state_root(&state);
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_state_root_order_independent() {
        let mut state1 = BTreeMap::new();
        state1.insert("alice".into(), 100);
        state1.insert("bob".into(), 200);

        let mut state2 = BTreeMap::new();
        state2.insert("bob".into(), 200);
        state2.insert("alice".into(), 100);

        assert_eq!(compute_state_root(&state1), compute_state_root(&state2));
    }

    #[test]
    fn test_trace_builder() {
        let mut initial_state = BTreeMap::new();
        initial_state.insert("alice".into(), 1000);

        let mut builder = TraceBuilder::new("batch_1".into(), &initial_state);

        builder.add_entry(
            0,
            TransactionId::new("tx_1"),
            "pre_hash".into(),
            "transfer".into(),
            vec!["alice".into()],
            vec!["alice".into()],
            "post_hash".into(),
            5000,
            true,
            None,
        );

        let mut final_state = initial_state.clone();
        final_state.insert("alice".into(), 900);

        let trace = builder.build(&final_state);

        assert_eq!(trace.batch_id, "batch_1");
        assert_eq!(trace.entries.len(), 1);
        assert!(!trace.trace_hash.is_empty());
        assert_ne!(trace.state_root_pre, trace.state_root_post);
        assert_eq!(trace.total_compute, 5000);
        assert_eq!(trace.success_count, 1);
    }
}
