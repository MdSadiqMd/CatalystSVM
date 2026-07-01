//! Batch sealing policies for the CatalystSVM pipeline
//! Policies decide when to seal a batch based on queue length, time, and latency
use catalyst_common::PolicyEngine;

/// Fixed-count policy: seal when queue reaches N transactions
#[derive(Debug)]
pub struct FixedCountPolicy {
    threshold: usize,
}

impl FixedCountPolicy {
    pub fn new(threshold: usize) -> Self {
        Self { threshold }
    }
}

impl PolicyEngine for FixedCountPolicy {
    fn should_seal(&self, queue_len: usize, _oldest_wait_ms: u64, _avg_arrival_rate: f64) -> bool {
        queue_len >= self.threshold
    }

    fn name(&self) -> &'static str {
        "FixedCount"
    }
}

/// Fixed-time policy: seal after T milliseconds since oldest transaction
#[derive(Debug)]
pub struct FixedTimePolicy {
    interval_ms: u64,
}

impl FixedTimePolicy {
    pub fn new(interval_ms: u64) -> Self {
        Self { interval_ms }
    }
}

impl PolicyEngine for FixedTimePolicy {
    fn should_seal(&self, queue_len: usize, oldest_wait_ms: u64, _avg_arrival_rate: f64) -> bool {
        queue_len > 0 && oldest_wait_ms >= self.interval_ms
    }

    fn name(&self) -> &'static str {
        "FixedTime"
    }
}

/// Hybrid policy: seal on count OR time, whichever comes first
#[derive(Debug)]
pub struct HybridPolicy {
    count_threshold: usize,
    time_threshold_ms: u64,
}

impl HybridPolicy {
    pub fn new(count_threshold: usize, time_threshold_ms: u64) -> Self {
        Self {
            count_threshold,
            time_threshold_ms,
        }
    }
}

impl PolicyEngine for HybridPolicy {
    fn should_seal(&self, queue_len: usize, oldest_wait_ms: u64, _avg_arrival_rate: f64) -> bool {
        queue_len >= self.count_threshold
            || (queue_len > 0 && oldest_wait_ms >= self.time_threshold_ms)
    }

    fn name(&self) -> &'static str {
        "Hybrid"
    }
}

/// Adaptive policy: proportional controller over queue-growth and latency-budget.
/// This is the project's novel contribution — adjusts batch threshold dynamically.
#[derive(Debug)]
pub struct AdaptivePolicy {
    base_count: usize,
    max_count: usize,
    min_count: usize,
    time_threshold_ms: u64,
    latency_budget_ms: u64,
    observed_latency_ms: u64,
    queue_growth_rate: f64,
}

impl AdaptivePolicy {
    pub fn new(
        base_count: usize,
        max_count: usize,
        time_threshold_ms: u64,
        latency_budget_ms: u64,
    ) -> Self {
        Self {
            base_count,
            max_count,
            min_count: 1,
            time_threshold_ms,
            latency_budget_ms,
            observed_latency_ms: 0,
            queue_growth_rate: 1.0,
        }
    }

    /// Compute dynamic threshold based on current conditions.
    fn compute_threshold(&self) -> usize {
        let mut threshold = self.base_count as f64;

        // Scale up if queue is growing fast (high arrival rate)
        if self.queue_growth_rate > 1.5 {
            threshold *= 1.0 + (self.queue_growth_rate - 1.0) * 0.3;
        }

        // Scale down if latency is approaching budget (proportional control)
        if self.observed_latency_ms > 0 {
            let latency_ratio = self.observed_latency_ms as f64 / self.latency_budget_ms as f64;
            if latency_ratio > 0.7 {
                // Aggressively shrink when nearing budget
                let shrink_factor = 1.0 - (latency_ratio - 0.7) * 2.0;
                threshold *= shrink_factor.max(0.3);
            }
        }

        (threshold.ceil() as usize)
            .max(self.min_count)
            .min(self.max_count)
    }
}

impl PolicyEngine for AdaptivePolicy {
    fn should_seal(&self, queue_len: usize, oldest_wait_ms: u64, avg_arrival_rate: f64) -> bool {
        if queue_len == 0 {
            return false;
        }

        let dynamic_threshold = self.compute_threshold();

        // Seal if dynamic threshold reached
        if queue_len >= dynamic_threshold {
            return true;
        }

        // Seal if time threshold exceeded
        if oldest_wait_ms >= self.time_threshold_ms {
            return true;
        }

        // Seal immediately if latency budget is breached
        if oldest_wait_ms >= self.latency_budget_ms {
            return true;
        }

        // Seal if queue is growing very fast and we have enough transactions
        if avg_arrival_rate > 100.0 && queue_len >= self.min_count {
            return true;
        }

        false
    }

    fn name(&self) -> &'static str {
        "Adaptive"
    }

    fn observe_latency(&mut self, latency_ms: u64) {
        // Exponential moving average for smoothing
        self.observed_latency_ms = (self.observed_latency_ms * 7 + latency_ms * 3) / 10;
    }

    fn observe_queue_growth(&mut self, rate: f64) {
        self.queue_growth_rate = (self.queue_growth_rate * 0.8) + (rate * 0.2);
    }
}

/// Priority-aware policy: fast-lane for critical transactions
#[derive(Debug)]
pub struct PriorityAwarePolicy {
    base_policy: HybridPolicy,
    critical_count_threshold: usize,
}

impl PriorityAwarePolicy {
    pub fn new(
        count_threshold: usize,
        time_threshold_ms: u64,
        critical_count_threshold: usize,
    ) -> Self {
        Self {
            base_policy: HybridPolicy::new(count_threshold, time_threshold_ms),
            critical_count_threshold,
        }
    }

    /// Check if should seal based on critical transaction count
    pub fn should_seal_for_critical(&self, critical_count: usize) -> bool {
        critical_count >= self.critical_count_threshold
    }
}

impl PolicyEngine for PriorityAwarePolicy {
    fn should_seal(&self, queue_len: usize, oldest_wait_ms: u64, avg_arrival_rate: f64) -> bool {
        self.base_policy
            .should_seal(queue_len, oldest_wait_ms, avg_arrival_rate)
    }

    fn name(&self) -> &'static str {
        "PriorityAware"
    }
}

/// Factory for creating policies by name
pub fn create_policy(name: &str, config: &PolicyConfig) -> Box<dyn PolicyEngine> {
    match name.to_lowercase().as_str() {
        "fixedcount" | "fixed_count" => Box::new(FixedCountPolicy::new(config.count_threshold)),
        "fixedtime" | "fixed_time" => Box::new(FixedTimePolicy::new(config.time_threshold_ms)),
        "hybrid" => Box::new(HybridPolicy::new(
            config.count_threshold,
            config.time_threshold_ms,
        )),
        "adaptive" => Box::new(AdaptivePolicy::new(
            config.count_threshold,
            config.max_batch_size,
            config.time_threshold_ms,
            config.latency_budget_ms,
        )),
        "priority" | "priorityaware" => Box::new(PriorityAwarePolicy::new(
            config.count_threshold,
            config.time_threshold_ms,
            config.critical_threshold,
        )),
        _ => Box::new(HybridPolicy::new(
            config.count_threshold,
            config.time_threshold_ms,
        )),
    }
}

/// Configuration for policy creation
#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub count_threshold: usize,
    pub time_threshold_ms: u64,
    pub latency_budget_ms: u64,
    pub max_batch_size: usize,
    pub critical_threshold: usize,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            count_threshold: 100,
            time_threshold_ms: 500,
            latency_budget_ms: 1000,
            max_batch_size: 500,
            critical_threshold: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_count_policy() {
        let policy = FixedCountPolicy::new(10);
        assert!(!policy.should_seal(5, 1000, 1.0));
        assert!(policy.should_seal(10, 0, 1.0));
        assert!(policy.should_seal(15, 0, 1.0));
    }

    #[test]
    fn test_fixed_time_policy() {
        let policy = FixedTimePolicy::new(100);
        assert!(!policy.should_seal(0, 200, 1.0)); // empty queue
        assert!(!policy.should_seal(5, 50, 1.0));
        assert!(policy.should_seal(5, 100, 1.0));
        assert!(policy.should_seal(1, 200, 1.0));
    }

    #[test]
    fn test_hybrid_policy() {
        let policy = HybridPolicy::new(10, 100);
        assert!(!policy.should_seal(5, 50, 1.0));
        assert!(policy.should_seal(10, 0, 1.0)); // count hit
        assert!(policy.should_seal(5, 100, 1.0)); // time hit
    }

    #[test]
    fn test_adaptive_policy_basic() {
        let policy = AdaptivePolicy::new(10, 50, 100, 500);
        assert!(!policy.should_seal(0, 0, 1.0));
        assert!(policy.should_seal(10, 0, 1.0));
        assert!(policy.should_seal(5, 500, 1.0)); // latency budget breach
    }

    #[test]
    fn test_adaptive_policy_latency_pressure() {
        let mut policy = AdaptivePolicy::new(10, 50, 100, 500);
        // Need multiple observations due to EMA smoothing
        for _ in 0..10 {
            policy.observe_latency(450); // 90% of budget
        }

        // Threshold should shrink, sealing at fewer txs
        let threshold = policy.compute_threshold();
        assert!(threshold < 10, "expected threshold < 10, got {}", threshold);
    }

    #[test]
    fn test_policy_factory() {
        let config = PolicyConfig::default();
        let hybrid = create_policy("hybrid", &config);
        assert_eq!(hybrid.name(), "Hybrid");

        let adaptive = create_policy("adaptive", &config);
        assert_eq!(adaptive.name(), "Adaptive");
    }
}
