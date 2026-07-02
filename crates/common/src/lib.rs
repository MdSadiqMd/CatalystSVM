//! Core types, traits, and utilities for the CatalystSVM batching pipeline.
//!
//! This crate provides the shared foundation used by all pipeline stages:
//! - Transaction/batch data structures
//! - Typed errors for each pipeline stage
//! - Hashing/Merkle utilities for deterministic commitments
//! - Virtual/system clock abstraction for reproducible simulations
//! - Metrics aggregation for benchmarking
//! - Trait interfaces (PolicyEngine, Executor, Prover, Verifier)

pub mod clock;
pub mod errors;
pub mod merkle;
pub mod metrics;
pub mod types;

pub use clock::{Clock, SystemClock, VirtualClock};
pub use errors::{ExecError, IngressError, PipelineError, ProveError, VerifyError};
pub use merkle::{hash_bytes, hash_str, hash_trace_entry, merkle_root, state_root};
pub use metrics::{MetricsAggregator, aggregate_metrics};
pub use types::{
    Batch, BatchMetrics, BatchStatus, ExecutionTrace, Proof, ProofCommitment, SealReason,
    SystemMetrics, TraceEntry, Transaction, TransactionId, TransactionPriority, VerificationResult,
};

/// Policy engine trait — sync and object-safe for runtime switching
pub trait PolicyEngine: Send + Sync {
    /// Returns true if the batch should be sealed now
    fn should_seal(&self, queue_len: usize, oldest_wait_ms: u64, avg_arrival_rate: f64) -> bool;

    /// Human-readable policy name
    fn name(&self) -> &'static str;

    /// Optional: notify policy of current latency for adaptive adjustment
    fn observe_latency(&mut self, _latency_ms: u64) {}

    /// Optional: notify policy of queue growth rate
    fn observe_queue_growth(&mut self, _rate: f64) {}
}

/// Executor trait — executes a batch of transactions deterministically
pub trait Executor: Send + Sync {
    fn execute(&self, batch: &Batch, clock: &dyn Clock) -> Result<ExecutionTrace, ExecError>;
}

/// Prover trait — generates a proof artifact from an execution trace and its transactions
pub trait Prover: Send + Sync {
    fn prove(
        &self,
        trace: &ExecutionTrace,
        transactions: &[Transaction],
        clock: &dyn Clock,
    ) -> Result<Proof, ProveError>;
}

/// Verifier trait — verifies a proof against an execution trace
pub trait Verifier: Send + Sync {
    fn verify(
        &self,
        proof: &Proof,
        trace: &ExecutionTrace,
        clock: &dyn Clock,
    ) -> Result<VerificationResult, VerifyError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_determinism() {
        let data = b"test data for hashing";
        let h1 = hash_bytes(data);
        let h2 = hash_bytes(data);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn test_merkle_root_determinism() {
        let leaves = vec![hash_str("a"), hash_str("b"), hash_str("c")];
        let r1 = merkle_root(&leaves);
        let r2 = merkle_root(&leaves);
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_merkle_root_empty() {
        let leaves: Vec<String> = vec![];
        let root = merkle_root(&leaves);
        assert!(!root.is_empty());
    }

    #[test]
    fn test_merkle_root_single() {
        let leaf = hash_str("single");
        let root = merkle_root(&[leaf.clone()]);
        assert_eq!(root, leaf);
    }
}
