# Benchmark Output Reference

This document explains the output of `just benchmark` (or `cargo run -p catalyst-benchmark -- compare`).

## Table Columns

| Column | Description |
|--------|-------------|
| **Policy** | Batching policy used. See [Policies](#policies) below. |
| **Scenario** | Workload pattern simulated. See [Scenarios](#scenarios) below. |
| **Txs** | Total transactions processed in the simulation. |
| **Batches** | Number of batches sealed. Fewer batches = better amortization of proving overhead. |
| **Avg Size** | Average transactions per batch. Higher = more efficient batching. |
| **P50 (ms)** | Median end-to-end latency per batch (ingest → verify). |
| **P95 (ms)** | 95th percentile latency. Important for SLA guarantees. |
| **TPS** | Average throughput in transactions per second. |
| **Proof (KB)** | Total proof data generated. Larger batches amortize fixed proof overhead. |
| **SLA Viol** | Batches exceeding the SLA latency threshold (default 200ms). |
| **Efficiency** | Ratio of execution time to total pipeline time. Higher = less proving overhead. |

## Policies

| Policy | Behavior |
|--------|----------|
| **fixedcount** | Seals batch when queue reaches N transactions (default 100). Ignores time. |
| **fixedtime** | Seals batch after T milliseconds (default 500ms). Ignores queue size. |
| **hybrid** | Seals on count OR time, whichever comes first. |
| **adaptive** | Dynamically adjusts threshold based on queue growth rate and latency budget. The project's novel contribution. |

## Scenarios

| Scenario | Pattern | Characteristics |
|----------|---------|-----------------|
| **steady** | Uniform arrivals | ~50ms between transactions. Baseline workload. |
| **bursty** | 20% bursts | Most transactions arrive slowly, 20% arrive in rapid bursts (1-5ms). |
| **poisson** | Exponential inter-arrival | Realistic random arrivals following Poisson process. |
| **mixed_priority** | Priority distribution | 5% critical, 15% high, 50% normal, 30% low priority transactions. |
| **idle_then_spike** | Three phases | 30% idle (100-500ms gaps), 40% spike (1-5ms), 30% cooldown. |

## Interpreting Results

### Tradeoff: Latency vs Throughput vs Proof Size

There are three competing concerns:

1. **Low latency** requires sealing batches quickly → smaller batches → more proofs → higher overhead
2. **High throughput** requires larger batches → more transactions per proof → but transactions wait longer
3. **Small proof size** requires fewer, larger batches → conflicts with low latency

### Policy Comparison Guide

**fixedcount (100 txs/batch)**
- Consistent batch sizes regardless of arrival rate
- Good throughput (151 TPS) but high latency (662ms P50)
- 100% SLA violations because batches always wait for 100 txs
- Best for: high-volume steady workloads where latency is not critical

**fixedtime (500ms)**
- Seals every 500ms regardless of queue size
- Lower latency (66-174ms P50) but variable batch sizes
- Poor efficiency on idle_then_spike (0.06) due to many tiny batches
- Best for: latency-sensitive workloads with predictable arrival

**hybrid**
- Combines count and time triggers
- Similar to fixedtime under low load, similar to fixedcount under high load
- Good general-purpose choice
- Best for: mixed workloads, default choice

**adaptive**
- Shrinks threshold when latency budget is tight
- Creates many small batches (avg 1.0-3.0 txs) to stay under latency
- Very low latency (66ms P50) but poor efficiency (0.03-0.08)
- Zero SLA violations on most scenarios
- Best for: strict latency SLAs, variable workloads

### Reading the Numbers

**Good fixedcount result (steady):**
```
fixedcount │ steady │ 1000 │ 10 │ 100.0 │ 662.0 │ 662.0 │ 151.1 │ 33.8 │ 10 │ 0.26
```
- 10 batches of exactly 100 txs each
- 662ms latency (bad) but 151 TPS throughput (good)
- 33.8 KB total proof (efficient amortization)
- 10 SLA violations (every batch exceeded 200ms)
- 0.26 efficiency (26% time spent on execution, 74% on proving/verifying)

**Good adaptive result (bursty):**
```
adaptive │ bursty │ 1000 │ 328 │ 3.0 │ 66.0 │ 108.0 │ 33.5 │ 113.2 │ 0 │ 0.08
```
- 328 batches averaging 3 txs each
- 66ms P50 latency (excellent), 108ms P95 (good)
- 33.5 TPS (lower due to small batches)
- 113.2 KB total proof (3x more than fixedcount due to per-batch overhead)
- 0 SLA violations
- 0.08 efficiency (8% execution, 92% overhead)

### Key Insights

1. **Adaptive wins on latency, loses on efficiency.** The current implementation aggressively seals to stay under budget, creating many small batches. Future work: tune the proportional controller gains.

2. **idle_then_spike breaks fixedtime and hybrid.** The 30% idle phase creates many 1-2 tx batches, destroying efficiency. Adaptive handles this better by adjusting its threshold.

3. **Proof size scales linearly with batch count.** Each batch has ~280 bytes fixed overhead plus ~35 bytes per transaction. Fewer batches = smaller total proof.

4. **Efficiency = execution_time / (execution + proving + verification).** Values below 0.10 mean >90% of time is spent on proving overhead, not useful work.

## Output Files

| File | Format | Contents |
|------|--------|----------|
| `results.json` | JSON array | Full metrics for each policy×scenario combination |
| `results.csv` | CSV | Same data in tabular format for spreadsheet analysis |
| `latency.png` | PNG chart | P95 latency comparison across policies |
| `throughput.png` | PNG chart | TPS comparison across policies |
| `proof_size.png` | PNG chart | Average proof size by policy |

## CLI Options

```bash
just compare seed=42 tx=1000 out=out     # Compare all policies
just simulate policy=adaptive scenario=bursty seed=42 tx=500  # Single run
just chart input=out/results.json out=out  # Generate charts from JSON
```

Key flags:
- `--seed`: RNG seed for reproducibility. Same seed = identical results.
- `--tx-count`: Number of transactions to simulate.
- `--batch-threshold`: Count threshold for fixedcount/hybrid (default 100).
- `--time-threshold-ms`: Time threshold for fixedtime/hybrid (default 500).
- `--latency-budget-ms`: Target latency for adaptive policy (default 1000).
- `--sla-latency-ms`: SLA threshold for violation counting (default 200).
