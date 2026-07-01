//! Pipeline orchestration engine for batching, execution, proving, and verification
use catalyst_common::{
    Batch, BatchMetrics, Clock, ExecError, MetricsAggregator, PolicyEngine, Prover, SealReason,
    SystemMetrics, Transaction, TransactionPriority, Verifier,
};
use catalyst_execution_engine::FullExecutor;
use std::collections::VecDeque;

/// Pipeline configuration.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PipelineConfig {
    pub batch_interval_ms: u64,
    pub sla_latency_ms: u64,
}

/// Orchestrates the full pipeline: ingest → batch → execute → prove → verify
pub struct PipelineEngine<'a, P, V>
where
    P: Prover,
    V: Verifier,
{
    policy: Box<dyn PolicyEngine>,
    executor: FullExecutor,
    prover: P,
    verifier: V,
    clock: &'a dyn Clock,
    #[allow(dead_code)]
    config: PipelineConfig,

    queue: VecDeque<Transaction>,
    priority_lane: VecDeque<Transaction>,
    batch_counter: u64,
    oldest_arrival: u64,
    last_seal_time: u64,

    metrics: MetricsAggregator,
}

impl<'a, P, V> PipelineEngine<'a, P, V>
where
    P: Prover,
    V: Verifier,
{
    pub fn new(
        policy: Box<dyn PolicyEngine>,
        executor: FullExecutor,
        prover: P,
        verifier: V,
        clock: &'a dyn Clock,
        config: PipelineConfig,
    ) -> Self {
        Self {
            policy,
            executor,
            prover,
            verifier,
            clock,
            config,
            queue: VecDeque::new(),
            priority_lane: VecDeque::new(),
            batch_counter: 0,
            oldest_arrival: 0,
            last_seal_time: 0,
            metrics: MetricsAggregator::new(),
        }
    }

    /// Ingest a transaction into the queue
    pub fn ingest(&mut self, tx: Transaction) -> Result<(), ExecError> {
        let arrival = tx.arrival_ts;

        if self.queue.is_empty() && self.priority_lane.is_empty() {
            self.oldest_arrival = arrival;
        }

        // Advance clock to transaction arrival time
        let current = self.clock.now_ms();
        if arrival > current {
            self.clock.advance(arrival - current);
        }

        // Route critical/high priority to fast lane
        if tx.priority >= TransactionPriority::High {
            self.priority_lane.push_back(tx);
        } else {
            self.queue.push_back(tx);
        }

        // Check if we should seal
        self.try_seal()?;

        Ok(())
    }

    /// Check sealing conditions and seal if needed
    fn try_seal(&mut self) -> Result<(), ExecError> {
        let total_queue_len = self.queue.len() + self.priority_lane.len();
        if total_queue_len == 0 {
            return Ok(());
        }

        let now = self.clock.now_ms();
        let oldest_wait = now.saturating_sub(self.oldest_arrival);

        // Compute arrival rate
        let time_since_last = now.saturating_sub(self.last_seal_time).max(1);
        let avg_arrival_rate = total_queue_len as f64 / (time_since_last as f64 / 1000.0);

        if self
            .policy
            .should_seal(total_queue_len, oldest_wait, avg_arrival_rate)
        {
            self.seal_and_process()?;
        }

        Ok(())
    }

    /// Seal the current batch and run the full pipeline
    fn seal_and_process(&mut self) -> Result<(), ExecError> {
        let now = self.clock.now_ms();

        // Drain priority lane first, then regular queue
        let mut transactions: Vec<Transaction> = self.priority_lane.drain(..).collect();
        transactions.extend(self.queue.drain(..));

        if transactions.is_empty() {
            return Ok(());
        }

        self.batch_counter += 1;
        let batch_id = format!("batch_{:06}", self.batch_counter);

        let tx_ids: Vec<_> = transactions.iter().map(|t| t.tx_id.clone()).collect();
        let batch_size = transactions.len();
        let batch_start = now;

        let mut batch = Batch::new(batch_id.clone(), batch_start);
        batch.tx_ids = tx_ids;
        batch.seal_ts = now;
        batch.reason_sealed = SealReason::Adaptive;

        // Execute
        let trace = self
            .executor
            .execute_with_transactions(&batch, &transactions, self.clock)?;

        let exec_time = trace.total_compute / 1000; // modeled: 1ms per 1000 CU

        // Prove
        let proof = self
            .prover
            .prove(&trace, &transactions, self.clock)
            .map_err(|e| ExecError::BatchFailed(format!("prove failed: {}", e)))?;

        // Verify
        let verification = self
            .verifier
            .verify(&proof, &trace, self.clock)
            .map_err(|e| ExecError::BatchFailed(format!("verify failed: {}", e)))?;

        let batch_end = self.clock.now_ms();
        let end_to_end = batch_end.saturating_sub(batch_start);

        // Compute throughput
        let throughput = if end_to_end > 0 {
            batch_size as f64 / (end_to_end as f64 / 1000.0)
        } else {
            batch_size as f64 * 1000.0
        };

        let batch_metrics = BatchMetrics {
            batch_id,
            policy_name: self.policy.name().to_string(),
            batch_size,
            total_compute_used: trace.total_compute,
            execution_time_ms: exec_time,
            proving_time_ms: proof.proving_time_ms,
            verification_time_ms: verification.verification_time_ms,
            end_to_end_latency_ms: end_to_end,
            proof_size_bytes: proof.proof_size_bytes as u64,
            throughput_tps: throughput,
            avg_tx_latency_ms: end_to_end as f64 / batch_size as f64,
            verification_passed: verification.is_valid,
        };

        self.metrics.record(batch_metrics);
        self.last_seal_time = batch_end;

        // Reset oldest arrival for next batch
        if let Some(next) = self.queue.front().or(self.priority_lane.front()) {
            self.oldest_arrival = next.arrival_ts;
        }

        Ok(())
    }

    /// Flush any remaining transactions
    pub fn flush(&mut self) -> Result<(), ExecError> {
        while !self.queue.is_empty() || !self.priority_lane.is_empty() {
            self.seal_and_process()?;
        }
        Ok(())
    }

    /// Get aggregated system metrics
    pub fn finalize(
        &self,
        policy_name: &str,
        scenario_name: &str,
        sla_latency_ms: u64,
    ) -> SystemMetrics {
        self.metrics
            .finalize(policy_name, scenario_name, sla_latency_ms)
    }
}
