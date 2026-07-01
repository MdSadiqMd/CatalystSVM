use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransactionId(pub String);

impl TransactionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TransactionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransactionPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for TransactionPriority {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub tx_id: TransactionId,
    pub arrival_ts: u64,
    pub sender: String,
    pub program_id: String,
    pub accounts: Vec<String>,
    pub instruction_data: Vec<u8>,
    pub priority: TransactionPriority,
    pub estimated_cu: u64,
    pub deadline_ms: Option<u64>,
}

impl Transaction {
    pub fn new(
        tx_id: TransactionId,
        sender: impl Into<String>,
        program_id: impl Into<String>,
    ) -> Self {
        Self {
            tx_id,
            arrival_ts: 0,
            sender: sender.into(),
            program_id: program_id.into(),
            accounts: Vec::new(),
            instruction_data: Vec::new(),
            priority: TransactionPriority::default(),
            estimated_cu: 200_000, // Solana default
            deadline_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BatchStatus {
    Pending,
    Sealed,
    Executed,
    ProofGenerated,
    Verified,
    Failed,
}

impl Default for BatchStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    pub batch_id: String,
    pub tx_ids: Vec<TransactionId>,
    pub start_ts: u64,
    pub seal_ts: u64,
    pub reason_sealed: SealReason,
    pub batch_hash: String,
    pub trace_hash: Option<String>,
    pub proof_hash: Option<String>,
    pub status: BatchStatus,
    pub total_cu: u64,
}

impl Batch {
    pub fn new(batch_id: impl Into<String>, start_ts: u64) -> Self {
        Self {
            batch_id: batch_id.into(),
            tx_ids: Vec::new(),
            start_ts,
            seal_ts: 0,
            reason_sealed: SealReason::Manual,
            batch_hash: String::new(),
            trace_hash: None,
            proof_hash: None,
            status: BatchStatus::Pending,
            total_cu: 0,
        }
    }

    pub fn size(&self) -> usize {
        self.tx_ids.len()
    }
}

/// Reason why a batch was sealed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SealReason {
    CountThreshold,
    TimeThreshold,
    LatencyBudget,
    PriorityFastLane,
    Manual,
    Adaptive,
}

impl std::fmt::Display for SealReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CountThreshold => write!(f, "count_threshold"),
            Self::TimeThreshold => write!(f, "time_threshold"),
            Self::LatencyBudget => write!(f, "latency_budget"),
            Self::PriorityFastLane => write!(f, "priority_fast_lane"),
            Self::Manual => write!(f, "manual"),
            Self::Adaptive => write!(f, "adaptive"),
        }
    }
}

/// A single entry in the execution trace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    pub tx_idx: usize,
    pub tx_id: TransactionId,
    pub pre_state_hash: String,
    pub instruction: String,
    pub accounts_read: Vec<String>,
    pub accounts_written: Vec<String>,
    pub post_state_hash: String,
    pub compute_used: u64,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub batch_id: String,
    pub entries: Vec<TraceEntry>,
    pub state_root_pre: String,
    pub state_root_post: String,
    pub trace_hash: String,
    pub total_compute: u64,
    pub success_count: usize,
    pub failure_count: usize,
}

impl ExecutionTrace {
    pub fn new(batch_id: impl Into<String>, state_root_pre: String) -> Self {
        Self {
            batch_id: batch_id.into(),
            entries: Vec::new(),
            state_root_pre,
            state_root_post: String::new(),
            trace_hash: String::new(),
            total_compute: 0,
            success_count: 0,
            failure_count: 0,
        }
    }
}

/// A proof artifact for a batch execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proof {
    pub batch_id: String,
    pub proof_data: Vec<u8>,
    pub proof_hash: String,
    pub commitment: ProofCommitment,
    pub generated_at: u64,
    pub proving_time_ms: u64,
    pub proof_size_bytes: usize,
}

/// The cryptographic commitment embedded in a proof
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofCommitment {
    pub batch_hash: String,
    pub trace_hash: String,
    pub state_root_pre: String,
    pub state_root_post: String,
    pub total_compute: u64,
    pub tx_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub batch_id: String,
    pub is_valid: bool,
    pub verification_time_ms: u64,
    pub error: Option<String>,
}

/// Metrics for a single batch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMetrics {
    pub batch_id: String,
    pub policy_name: String,
    pub batch_size: usize,
    pub total_compute_used: u64,
    pub execution_time_ms: u64,
    pub proving_time_ms: u64,
    pub verification_time_ms: u64,
    pub end_to_end_latency_ms: u64,
    pub proof_size_bytes: u64,
    pub throughput_tps: f64,
    pub avg_tx_latency_ms: f64,
    pub verification_passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub policy_name: String,
    pub scenario_name: String,
    pub total_transactions: usize,
    pub total_batches: usize,
    pub avg_batch_size: f64,
    pub avg_latency_ms: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub avg_throughput_tps: f64,
    pub total_proof_size_bytes: u64,
    pub avg_proof_size_bytes: f64,
    pub verification_success_rate: f64,
    pub total_compute_used: u64,
    pub amortized_cost_per_tx: f64,
    pub batch_efficiency: f64,
    pub sla_violations: usize,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            policy_name: String::new(),
            scenario_name: String::new(),
            total_transactions: 0,
            total_batches: 0,
            avg_batch_size: 0.0,
            avg_latency_ms: 0.0,
            p50_latency_ms: 0.0,
            p95_latency_ms: 0.0,
            p99_latency_ms: 0.0,
            avg_throughput_tps: 0.0,
            total_proof_size_bytes: 0,
            avg_proof_size_bytes: 0.0,
            verification_success_rate: 0.0,
            total_compute_used: 0,
            amortized_cost_per_tx: 0.0,
            batch_efficiency: 0.0,
            sla_violations: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_id_display() {
        let id = TransactionId::new("tx_123");
        assert_eq!(format!("{id}"), "tx_123");
    }

    #[test]
    fn test_priority_ordering() {
        assert!(TransactionPriority::Critical > TransactionPriority::High);
        assert!(TransactionPriority::High > TransactionPriority::Normal);
        assert!(TransactionPriority::Normal > TransactionPriority::Low);
    }

    #[test]
    fn test_batch_size() {
        let mut batch = Batch::new("batch_1", 1000);
        assert_eq!(batch.size(), 0);
        batch.tx_ids.push(TransactionId::new("tx_1"));
        batch.tx_ids.push(TransactionId::new("tx_2"));
        assert_eq!(batch.size(), 2);
    }

    #[test]
    fn test_seal_reason_display() {
        assert_eq!(SealReason::CountThreshold.to_string(), "count_threshold");
        assert_eq!(SealReason::Adaptive.to_string(), "adaptive");
    }
}
