use crate::types::{BatchMetrics, SystemMetrics};

pub fn aggregate_metrics(
    batch_metrics: &[BatchMetrics],
    policy_name: &str,
    scenario_name: &str,
    sla_latency_ms: u64,
) -> SystemMetrics {
    if batch_metrics.is_empty() {
        return SystemMetrics {
            policy_name: policy_name.to_string(),
            scenario_name: scenario_name.to_string(),
            ..Default::default()
        };
    }

    let total_batches = batch_metrics.len();
    let total_transactions: usize = batch_metrics.iter().map(|m| m.batch_size).sum();

    let mut latencies: Vec<f64> = batch_metrics
        .iter()
        .map(|m| m.end_to_end_latency_ms as f64)
        .collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let avg_latency_ms = latencies.iter().sum::<f64>() / latencies.len() as f64;
    let p50_latency_ms = percentile(&latencies, 50.0);
    let p95_latency_ms = percentile(&latencies, 95.0);
    let p99_latency_ms = percentile(&latencies, 99.0);

    let total_proof_size_bytes: u64 = batch_metrics.iter().map(|m| m.proof_size_bytes).sum();
    let avg_proof_size_bytes = total_proof_size_bytes as f64 / total_batches as f64;

    let throughputs: Vec<f64> = batch_metrics.iter().map(|m| m.throughput_tps).collect();
    let avg_throughput_tps = throughputs.iter().sum::<f64>() / throughputs.len() as f64;

    let verification_successes = batch_metrics
        .iter()
        .filter(|m| m.verification_passed)
        .count();
    let verification_success_rate = verification_successes as f64 / total_batches as f64;

    let total_compute_used: u64 = batch_metrics.iter().map(|m| m.total_compute_used).sum();

    let sla_violations = batch_metrics
        .iter()
        .filter(|m| m.end_to_end_latency_ms > sla_latency_ms)
        .count();

    let avg_batch_size = total_transactions as f64 / total_batches as f64;

    let amortized_cost_per_tx = if total_transactions > 0 {
        total_proof_size_bytes as f64 / total_transactions as f64
    } else {
        0.0
    };

    let batch_efficiency = compute_batch_efficiency(batch_metrics);

    SystemMetrics {
        policy_name: policy_name.to_string(),
        scenario_name: scenario_name.to_string(),
        total_transactions,
        total_batches,
        avg_batch_size,
        avg_latency_ms,
        p50_latency_ms,
        p95_latency_ms,
        p99_latency_ms,
        avg_throughput_tps,
        total_proof_size_bytes,
        avg_proof_size_bytes,
        verification_success_rate,
        total_compute_used,
        amortized_cost_per_tx,
        batch_efficiency,
        sla_violations,
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Batch efficiency: ratio of useful compute vs overhead
/// Higher is better — measures how well batches amortize fixed costs
fn compute_batch_efficiency(metrics: &[BatchMetrics]) -> f64 {
    if metrics.is_empty() {
        return 0.0;
    }

    let total_execution: u64 = metrics.iter().map(|m| m.execution_time_ms).sum();
    let total_overhead: u64 = metrics
        .iter()
        .map(|m| m.proving_time_ms + m.verification_time_ms)
        .sum();

    if total_overhead == 0 {
        return 1.0;
    }

    total_execution as f64 / (total_execution + total_overhead) as f64
}

/// Accumulator for collecting metrics during a simulation run
#[derive(Debug, Default)]
pub struct MetricsAggregator {
    batch_metrics: Vec<BatchMetrics>,
}

impl MetricsAggregator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, metrics: BatchMetrics) {
        self.batch_metrics.push(metrics);
    }

    pub fn batch_count(&self) -> usize {
        self.batch_metrics.len()
    }

    pub fn transaction_count(&self) -> usize {
        self.batch_metrics.iter().map(|m| m.batch_size).sum()
    }

    pub fn finalize(
        &self,
        policy_name: &str,
        scenario_name: &str,
        sla_latency_ms: u64,
    ) -> SystemMetrics {
        aggregate_metrics(
            &self.batch_metrics,
            policy_name,
            scenario_name,
            sla_latency_ms,
        )
    }

    pub fn batch_metrics(&self) -> &[BatchMetrics] {
        &self.batch_metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_batch_metrics(latency: u64, size: usize) -> BatchMetrics {
        BatchMetrics {
            batch_id: format!("batch_{}", latency),
            policy_name: "test".to_string(),
            batch_size: size,
            total_compute_used: size as u64 * 1000,
            execution_time_ms: 10,
            proving_time_ms: 50,
            verification_time_ms: 5,
            end_to_end_latency_ms: latency,
            proof_size_bytes: 1000,
            throughput_tps: size as f64 / (latency as f64 / 1000.0),
            avg_tx_latency_ms: latency as f64 / size as f64,
            verification_passed: true,
        }
    }

    #[test]
    fn test_aggregate_empty() {
        let result = aggregate_metrics(&[], "test", "empty", 100);
        assert_eq!(result.total_batches, 0);
        assert_eq!(result.total_transactions, 0);
    }

    #[test]
    fn test_aggregate_single() {
        let metrics = vec![make_batch_metrics(100, 10)];
        let result = aggregate_metrics(&metrics, "test", "single", 200);

        assert_eq!(result.total_batches, 1);
        assert_eq!(result.total_transactions, 10);
        assert_eq!(result.avg_latency_ms, 100.0);
        assert_eq!(result.p50_latency_ms, 100.0);
        assert_eq!(result.sla_violations, 0);
    }

    #[test]
    fn test_aggregate_multiple() {
        let metrics = vec![
            make_batch_metrics(50, 5),
            make_batch_metrics(100, 10),
            make_batch_metrics(150, 15),
            make_batch_metrics(200, 20),
        ];
        let result = aggregate_metrics(&metrics, "test", "multi", 120);

        assert_eq!(result.total_batches, 4);
        assert_eq!(result.total_transactions, 50);
        assert_eq!(result.sla_violations, 2); // 150 and 200 exceed 120ms SLA
    }

    #[test]
    fn test_percentile() {
        let sorted = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        // P50 with round((0.5 * 9)) = 5 → sorted[5] = 6
        assert_eq!(percentile(&sorted, 50.0), 6.0);
        assert_eq!(percentile(&sorted, 0.0), 1.0);
        assert_eq!(percentile(&sorted, 100.0), 10.0);
    }

    #[test]
    fn test_metrics_aggregator() {
        let mut agg = MetricsAggregator::new();
        agg.record(make_batch_metrics(100, 10));
        agg.record(make_batch_metrics(200, 20));

        assert_eq!(agg.batch_count(), 2);
        assert_eq!(agg.transaction_count(), 30);

        let result = agg.finalize("test", "agg_test", 150);
        assert_eq!(result.total_batches, 2);
        assert_eq!(result.sla_violations, 1);
    }
}
