use catalyst_common::{IngressError, Transaction, TransactionId, TransactionPriority};
use rand::prelude::*;
use rand_distr::Exp;

pub fn validate(tx: &Transaction) -> Result<(), IngressError> {
    if tx.sender.is_empty() {
        return Err(IngressError::InvalidSender("empty sender".into()));
    }

    if tx.program_id.is_empty() {
        return Err(IngressError::InvalidProgram("empty program_id".into()));
    }

    if tx.instruction_data.is_empty() {
        return Err(IngressError::EmptyInstruction);
    }

    const MAX_COMPUTE_BUDGET: u64 = 1_400_000;
    if tx.estimated_cu > MAX_COMPUTE_BUDGET {
        return Err(IngressError::ComputeBudgetExceeded {
            requested: tx.estimated_cu,
            max: MAX_COMPUTE_BUDGET,
        });
    }

    Ok(())
}

/// Workload scenario types for benchmarking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Steady,
    Bursty,
    Poisson,
    MixedPriority,
    IdleThenSpike,
}

impl Scenario {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::Bursty => "bursty",
            Self::Poisson => "poisson",
            Self::MixedPriority => "mixed_priority",
            Self::IdleThenSpike => "idle_then_spike",
        }
    }

    pub fn all() -> &'static [Scenario] {
        &[
            Scenario::Steady,
            Scenario::Bursty,
            Scenario::Poisson,
            Scenario::MixedPriority,
            Scenario::IdleThenSpike,
        ]
    }
}

impl std::str::FromStr for Scenario {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "steady" => Ok(Self::Steady),
            "bursty" => Ok(Self::Bursty),
            "poisson" => Ok(Self::Poisson),
            "mixed_priority" | "mixedpriority" => Ok(Self::MixedPriority),
            "idle_then_spike" | "idlethenspike" => Ok(Self::IdleThenSpike),
            _ => Err(format!("unknown scenario: {}", s)),
        }
    }
}

/// Seeded workload generator for deterministic benchmarks
pub struct WorkloadGenerator {
    rng: StdRng,
    scenario: Scenario,
    tx_count: usize,
    base_interval_ms: u64,
    current_time_ms: u64,
    generated: usize,
}

impl std::fmt::Debug for WorkloadGenerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkloadGenerator")
            .field("scenario", &self.scenario)
            .field("tx_count", &self.tx_count)
            .field("generated", &self.generated)
            .finish_non_exhaustive()
    }
}

impl WorkloadGenerator {
    pub fn new(scenario: Scenario, tx_count: usize, seed: u64, base_interval_ms: u64) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            scenario,
            tx_count,
            base_interval_ms,
            current_time_ms: 0,
            generated: 0,
        }
    }

    /// Generate the next transaction with its arrival timestamp
    pub fn next(&mut self) -> Option<Transaction> {
        if self.generated >= self.tx_count {
            return None;
        }

        let (delta_ms, priority) = self.compute_arrival();
        self.current_time_ms += delta_ms;
        let arrival_ts = self.current_time_ms;

        let tx_id = TransactionId::new(format!("tx_{:06}", self.generated));
        let sender = format!("sender_{}", self.rng.r#gen_range(0..100));
        let program_id = format!("program_{}", self.rng.r#gen_range(0..5));

        let estimated_cu = match priority {
            TransactionPriority::Critical => self.rng.r#gen_range(100_000..200_000),
            TransactionPriority::High => self.rng.r#gen_range(50_000..150_000),
            TransactionPriority::Normal => self.rng.r#gen_range(10_000..100_000),
            TransactionPriority::Low => self.rng.r#gen_range(5_000..50_000),
        };

        let instruction_data = self.generate_instruction();

        self.generated += 1;

        let mut tx = Transaction::new(tx_id, sender, program_id);
        tx.arrival_ts = arrival_ts;
        tx.instruction_data = instruction_data;
        tx.estimated_cu = estimated_cu;
        tx.priority = priority;

        Some(tx)
    }

    fn compute_arrival(&mut self) -> (u64, TransactionPriority) {
        match self.scenario {
            Scenario::Steady => {
                let jitter = self.rng.r#gen_range(0..10) as i64 - 5;
                let delta = (self.base_interval_ms as i64 + jitter).max(1) as u64;
                (delta, TransactionPriority::Normal)
            }
            Scenario::Bursty => {
                let is_burst = self.rng.r#gen_bool(0.2);
                let delta = if is_burst {
                    self.rng.r#gen_range(1..5)
                } else {
                    self.rng.r#gen_range(50..200)
                };
                (delta, TransactionPriority::Normal)
            }
            Scenario::Poisson => {
                let lambda = 1.0 / self.base_interval_ms as f64;
                let exp = Exp::new(lambda).unwrap_or_else(|_| Exp::new(0.01).unwrap());
                let delta = self.rng.sample(exp).max(1.0) as u64;
                (delta, TransactionPriority::Normal)
            }
            Scenario::MixedPriority => {
                let delta = self.rng.r#gen_range(5..50);
                let priority = match self.rng.r#gen_range(0..100) {
                    0..=5 => TransactionPriority::Critical,
                    6..=20 => TransactionPriority::High,
                    21..=70 => TransactionPriority::Normal,
                    _ => TransactionPriority::Low,
                };
                (delta, priority)
            }
            Scenario::IdleThenSpike => {
                let progress = self.generated as f64 / self.tx_count as f64;
                if progress < 0.3 {
                    // Idle phase: slow arrivals
                    (self.rng.r#gen_range(100..500), TransactionPriority::Low)
                } else if progress < 0.7 {
                    // Spike phase: rapid arrivals
                    (self.rng.r#gen_range(1..5), TransactionPriority::High)
                } else {
                    // Cool-down phase
                    (self.rng.r#gen_range(20..100), TransactionPriority::Normal)
                }
            }
        }
    }

    fn generate_instruction(&mut self) -> Vec<u8> {
        let opcode = self.rng.r#gen_range(0u8..5);
        let mut data = vec![opcode];

        match opcode {
            0 => {
                // Transfer: [opcode, amount (8 bytes), target (8 bytes)]
                let amount = self.rng.r#gen_range(1u64..1000);
                let target = self.rng.r#gen_range(0u64..100);
                data.extend_from_slice(&amount.to_le_bytes());
                data.extend_from_slice(&target.to_le_bytes());
            }
            1 => {
                // SetData: [opcode, key (8 bytes), value (8 bytes)]
                let key = self.rng.r#gen_range(0u64..50);
                let value = self.rng.r#gen::<u64>();
                data.extend_from_slice(&key.to_le_bytes());
                data.extend_from_slice(&value.to_le_bytes());
            }
            2 => {
                // Increment: [opcode, key (8 bytes), delta (8 bytes)]
                let key = self.rng.r#gen_range(0u64..50);
                let delta = self.rng.r#gen_range(1u64..100);
                data.extend_from_slice(&key.to_le_bytes());
                data.extend_from_slice(&delta.to_le_bytes());
            }
            3 => {
                // Noop: [opcode]
            }
            _ => {
                // Invalid (for testing failure paths): [opcode, garbage]
                data.extend_from_slice(&[0xFF; 8]);
            }
        }

        data
    }

    pub fn remaining(&self) -> usize {
        self.tx_count.saturating_sub(self.generated)
    }

    pub fn scenario(&self) -> Scenario {
        self.scenario
    }
}

impl Iterator for WorkloadGenerator {
    type Item = Transaction;

    fn next(&mut self) -> Option<Self::Item> {
        WorkloadGenerator::next(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_success() {
        let mut tx = Transaction::new(TransactionId::new("tx_1"), "alice", "program_1");
        tx.instruction_data = vec![0, 1, 2, 3];
        tx.estimated_cu = 100_000;

        assert!(validate(&tx).is_ok());
    }

    #[test]
    fn test_validate_empty_sender() {
        let mut tx = Transaction::new(TransactionId::new("tx_1"), "", "program_1");
        tx.instruction_data = vec![0];

        assert!(matches!(validate(&tx), Err(IngressError::InvalidSender(_))));
    }

    #[test]
    fn test_validate_compute_exceeded() {
        let mut tx = Transaction::new(TransactionId::new("tx_1"), "alice", "program_1");
        tx.instruction_data = vec![0];
        tx.estimated_cu = 2_000_000; // Exceeds 1.4M limit

        assert!(matches!(
            validate(&tx),
            Err(IngressError::ComputeBudgetExceeded { .. })
        ));
    }

    #[test]
    fn test_workload_generator_deterministic() {
        let gen1: Vec<_> = WorkloadGenerator::new(Scenario::Steady, 10, 42, 100).collect();
        let gen2: Vec<_> = WorkloadGenerator::new(Scenario::Steady, 10, 42, 100).collect();

        assert_eq!(gen1.len(), gen2.len());
        for (t1, t2) in gen1.iter().zip(gen2.iter()) {
            assert_eq!(t1.tx_id.as_str(), t2.tx_id.as_str());
            assert_eq!(t1.arrival_ts, t2.arrival_ts);
            assert_eq!(t1.estimated_cu, t2.estimated_cu);
        }
    }

    #[test]
    fn test_workload_generator_scenarios() {
        for scenario in Scenario::all() {
            let txs: Vec<_> = WorkloadGenerator::new(*scenario, 50, 123, 50).collect();
            assert_eq!(txs.len(), 50, "scenario {:?}", scenario);

            // Verify timestamps are monotonically increasing
            for i in 1..txs.len() {
                assert!(
                    txs[i].arrival_ts >= txs[i - 1].arrival_ts,
                    "scenario {:?} timestamp not monotonic",
                    scenario
                );
            }
        }
    }

    #[test]
    fn test_mixed_priority_has_variety() {
        let txs: Vec<_> = WorkloadGenerator::new(Scenario::MixedPriority, 200, 456, 20).collect();
        let priorities: std::collections::HashSet<_> = txs.iter().map(|t| t.priority).collect();
        assert!(priorities.len() > 1, "expected multiple priority levels");
    }

    #[test]
    fn test_scenario_from_str() {
        assert_eq!("steady".parse::<Scenario>().unwrap(), Scenario::Steady);
        assert_eq!("BURSTY".parse::<Scenario>().unwrap(), Scenario::Bursty);
        assert_eq!(
            "mixed_priority".parse::<Scenario>().unwrap(),
            Scenario::MixedPriority
        );
        assert!("unknown".parse::<Scenario>().is_err());
    }
}
