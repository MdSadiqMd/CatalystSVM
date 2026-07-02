//! CatalystSVM Benchmark CLI — compare batching policies across workloads
mod engine;

use anyhow::Result;
use catalyst_batch_policy::{PolicyConfig, create_policy};
use catalyst_common::{SystemMetrics, VirtualClock};
use catalyst_execution_engine::FullExecutor;
use catalyst_ingress::{Scenario, WorkloadGenerator};
use catalyst_sp1_prover::Sp1Prover;
use catalyst_sp1_verifier::Sp1Verifier;
use clap::{Parser, Subcommand};
use comfy_table::{Table, presets::UTF8_FULL};
use engine::{PipelineConfig, PipelineEngine};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "catalyst-benchmark")]
#[command(about = "Latency-aware zkSVM batcher benchmark tool")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a single policy on a single scenario
    Simulate {
        #[arg(short, long, default_value = "hybrid")]
        policy: String,

        #[arg(short, long, default_value = "steady")]
        scenario: String,

        #[arg(long, default_value = "42")]
        seed: u64,

        #[arg(long, default_value = "1000")]
        tx_count: usize,

        #[arg(long, default_value = "100")]
        batch_threshold: usize,

        #[arg(long, default_value = "500")]
        time_threshold_ms: u64,

        #[arg(long, default_value = "1000")]
        latency_budget_ms: u64,
    },

    /// Compare all policies across scenarios
    Compare {
        #[arg(long, default_value = "42")]
        seed: u64,

        #[arg(long, default_value = "500")]
        tx_count: usize,

        #[arg(short, long)]
        out: Option<PathBuf>,

        #[arg(long, default_value = "100")]
        batch_threshold: usize,

        #[arg(long, default_value = "500")]
        time_threshold_ms: u64,

        #[arg(long, default_value = "1000")]
        latency_budget_ms: u64,

        #[arg(long, default_value = "200")]
        sla_latency_ms: u64,
    },

    /// Generate charts from results JSON
    Chart {
        #[arg(short, long)]
        input: PathBuf,

        #[arg(short, long)]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("catalyst=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Simulate {
            policy,
            scenario,
            seed,
            tx_count,
            batch_threshold,
            time_threshold_ms,
            latency_budget_ms,
        } => {
            let scenario = scenario
                .parse::<Scenario>()
                .map_err(|e| anyhow::anyhow!(e))?;

            let config = PolicyConfig {
                count_threshold: batch_threshold,
                time_threshold_ms,
                latency_budget_ms,
                max_batch_size: batch_threshold * 5,
                critical_threshold: 5,
            };

            let result = run_simulation(
                &policy,
                scenario,
                seed,
                tx_count,
                &config,
                latency_budget_ms,
            )?;

            print_single_result(&result);
        }

        Commands::Compare {
            seed,
            tx_count,
            out,
            batch_threshold,
            time_threshold_ms,
            latency_budget_ms,
            sla_latency_ms,
        } => {
            let config = PolicyConfig {
                count_threshold: batch_threshold,
                time_threshold_ms,
                latency_budget_ms,
                max_batch_size: batch_threshold * 5,
                critical_threshold: 5,
            };

            let results = run_comparison(seed, tx_count, &config, sla_latency_ms)?;

            print_comparison_table(&results);

            if let Some(out_dir) = out {
                fs::create_dir_all(&out_dir)?;
                save_results(&results, &out_dir)?;
                println!("\nResults saved to {}", out_dir.display());
            }
        }

        Commands::Chart { input, out } => {
            generate_charts(&input, &out)?;
            println!("Charts saved to {}", out.display());
        }
    }

    Ok(())
}

fn run_simulation(
    policy_name: &str,
    scenario: Scenario,
    seed: u64,
    tx_count: usize,
    config: &PolicyConfig,
    sla_latency_ms: u64,
) -> Result<SystemMetrics> {
    let clock = VirtualClock::new(0);
    let policy = create_policy(policy_name, config);
    let executor = FullExecutor::new(200_000);
    let prover = Sp1Prover::new();
    let verifier = Sp1Verifier::new();

    let pipeline_config = PipelineConfig {
        batch_interval_ms: config.time_threshold_ms,
        sla_latency_ms,
    };

    let mut engine =
        PipelineEngine::new(policy, executor, prover, verifier, &clock, pipeline_config);

    let workload = WorkloadGenerator::new(scenario, tx_count, seed, 50);

    for tx in workload {
        engine.ingest(tx)?;
    }

    engine.flush()?;

    let metrics = engine.finalize(policy_name, scenario.name(), sla_latency_ms);

    Ok(metrics)
}

fn run_comparison(
    seed: u64,
    tx_count: usize,
    config: &PolicyConfig,
    sla_latency_ms: u64,
) -> Result<Vec<SystemMetrics>> {
    let policies = ["fixedcount", "fixedtime", "hybrid", "adaptive"];
    let scenarios = Scenario::all();

    let mut results = Vec::new();

    for policy in &policies {
        for scenario in scenarios {
            let result = run_simulation(policy, *scenario, seed, tx_count, config, sla_latency_ms)?;
            results.push(result);
        }
    }

    Ok(results)
}

fn print_single_result(metrics: &SystemMetrics) {
    println!("\n=== Simulation Results ===");
    println!("Policy: {}", metrics.policy_name);
    println!("Scenario: {}", metrics.scenario_name);
    println!("Total transactions: {}", metrics.total_transactions);
    println!("Total batches: {}", metrics.total_batches);
    println!("Avg batch size: {:.1}", metrics.avg_batch_size);
    println!("Avg latency: {:.2} ms", metrics.avg_latency_ms);
    println!("P50 latency: {:.2} ms", metrics.p50_latency_ms);
    println!("P95 latency: {:.2} ms", metrics.p95_latency_ms);
    println!("P99 latency: {:.2} ms", metrics.p99_latency_ms);
    println!("Avg throughput: {:.2} TPS", metrics.avg_throughput_tps);
    println!("Total proof size: {} bytes", metrics.total_proof_size_bytes);
    println!(
        "Verification success rate: {:.1}%",
        metrics.verification_success_rate * 100.0
    );
    println!("SLA violations: {}", metrics.sla_violations);
    println!("Batch efficiency: {:.2}", metrics.batch_efficiency);
}

fn print_comparison_table(results: &[SystemMetrics]) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        "Policy",
        "Scenario",
        "Txs",
        "Batches",
        "Avg Size",
        "P50 (ms)",
        "P95 (ms)",
        "TPS",
        "Proof (KB)",
        "SLA Viol",
        "Efficiency",
    ]);

    for m in results {
        table.add_row(vec![
            m.policy_name.clone(),
            m.scenario_name.clone(),
            m.total_transactions.to_string(),
            m.total_batches.to_string(),
            format!("{:.1}", m.avg_batch_size),
            format!("{:.1}", m.p50_latency_ms),
            format!("{:.1}", m.p95_latency_ms),
            format!("{:.1}", m.avg_throughput_tps),
            format!("{:.1}", m.total_proof_size_bytes as f64 / 1024.0),
            m.sla_violations.to_string(),
            format!("{:.2}", m.batch_efficiency),
        ]);
    }

    println!("\n{table}");
}

fn save_results(results: &[SystemMetrics], out_dir: &PathBuf) -> Result<()> {
    let json_path = out_dir.join("results.json");
    let json = serde_json::to_string_pretty(results)?;
    fs::write(&json_path, json)?;

    let csv_path = out_dir.join("results.csv");
    let mut wtr = csv::Writer::from_path(&csv_path)?;

    wtr.write_record([
        "policy",
        "scenario",
        "total_transactions",
        "total_batches",
        "avg_batch_size",
        "avg_latency_ms",
        "p50_latency_ms",
        "p95_latency_ms",
        "p99_latency_ms",
        "avg_throughput_tps",
        "total_proof_size_bytes",
        "verification_success_rate",
        "sla_violations",
        "batch_efficiency",
    ])?;

    for m in results {
        wtr.write_record([
            &m.policy_name,
            &m.scenario_name,
            &m.total_transactions.to_string(),
            &m.total_batches.to_string(),
            &format!("{:.2}", m.avg_batch_size),
            &format!("{:.2}", m.avg_latency_ms),
            &format!("{:.2}", m.p50_latency_ms),
            &format!("{:.2}", m.p95_latency_ms),
            &format!("{:.2}", m.p99_latency_ms),
            &format!("{:.2}", m.avg_throughput_tps),
            &m.total_proof_size_bytes.to_string(),
            &format!("{:.4}", m.verification_success_rate),
            &m.sla_violations.to_string(),
            &format!("{:.4}", m.batch_efficiency),
        ])?;
    }

    wtr.flush()?;

    Ok(())
}

fn generate_charts(input: &PathBuf, out_dir: &PathBuf) -> Result<()> {
    use plotters::prelude::*;

    fs::create_dir_all(out_dir)?;

    let json = fs::read_to_string(input)?;
    let results: Vec<SystemMetrics> = serde_json::from_str(&json)?;

    let policies: Vec<String> = results
        .iter()
        .map(|r| r.policy_name.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let scenarios: Vec<String> = results
        .iter()
        .map(|r| r.scenario_name.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Latency comparison chart
    let latency_path = out_dir.join("latency.png");
    {
        let root = BitMapBackend::new(&latency_path, (800, 600)).into_drawing_area();
        root.fill(&WHITE)?;

        let max_latency = results
            .iter()
            .map(|r| r.p95_latency_ms)
            .fold(0.0f64, |a, b| a.max(b))
            * 1.2;

        let mut chart = ChartBuilder::on(&root)
            .caption("P95 Latency by Policy and Scenario", ("sans-serif", 24))
            .margin(20)
            .x_label_area_size(60)
            .y_label_area_size(60)
            .build_cartesian_2d(0..(policies.len() * scenarios.len()), 0.0..max_latency)?;

        chart
            .configure_mesh()
            .y_desc("P95 Latency (ms)")
            .x_desc("Policy-Scenario")
            .draw()?;

        let colors = [&RED, &BLUE, &GREEN, &MAGENTA];

        for (i, policy) in policies.iter().enumerate() {
            let policy_results: Vec<_> = results
                .iter()
                .filter(|r| &r.policy_name == policy)
                .collect();

            let points: Vec<_> = policy_results
                .iter()
                .enumerate()
                .map(|(j, r)| (i * scenarios.len() + j, r.p95_latency_ms))
                .collect();

            chart
                .draw_series(
                    points
                        .iter()
                        .map(|(x, y)| Circle::new((*x, *y), 5, colors[i % colors.len()].filled())),
                )?
                .label(policy)
                .legend(move |(x, y)| Circle::new((x, y), 5, colors[i % colors.len()].filled()));
        }

        chart
            .configure_series_labels()
            .background_style(&WHITE.mix(0.8))
            .border_style(&BLACK)
            .draw()?;

        root.present()?;
    }

    // Throughput comparison chart
    let throughput_path = out_dir.join("throughput.png");
    {
        let root = BitMapBackend::new(&throughput_path, (800, 600)).into_drawing_area();
        root.fill(&WHITE)?;

        let max_tps = results
            .iter()
            .map(|r| r.avg_throughput_tps)
            .fold(0.0f64, |a, b| a.max(b))
            * 1.2;

        let mut chart = ChartBuilder::on(&root)
            .caption("Throughput by Policy and Scenario", ("sans-serif", 24))
            .margin(20)
            .x_label_area_size(60)
            .y_label_area_size(60)
            .build_cartesian_2d(0..(policies.len() * scenarios.len()), 0.0..max_tps)?;

        chart
            .configure_mesh()
            .y_desc("Throughput (TPS)")
            .x_desc("Policy-Scenario")
            .draw()?;

        let colors = [&RED, &BLUE, &GREEN, &MAGENTA];

        for (i, policy) in policies.iter().enumerate() {
            let policy_results: Vec<_> = results
                .iter()
                .filter(|r| &r.policy_name == policy)
                .collect();

            let points: Vec<_> = policy_results
                .iter()
                .enumerate()
                .map(|(j, r)| (i * scenarios.len() + j, r.avg_throughput_tps))
                .collect();

            chart
                .draw_series(points.iter().map(|(x, y)| {
                    Rectangle::new(
                        [(*x, 0.0), (*x + 1, *y)],
                        colors[i % colors.len()].mix(0.7).filled(),
                    )
                }))?
                .label(policy)
                .legend(move |(x, y)| {
                    Rectangle::new(
                        [(x - 5, y - 5), (x + 5, y + 5)],
                        colors[i % colors.len()].filled(),
                    )
                });
        }

        chart
            .configure_series_labels()
            .background_style(&WHITE.mix(0.8))
            .border_style(&BLACK)
            .draw()?;

        root.present()?;
    }

    // Proof size chart
    let proof_path = out_dir.join("proof_size.png");
    {
        let root = BitMapBackend::new(&proof_path, (800, 600)).into_drawing_area();
        root.fill(&WHITE)?;

        let max_size = results
            .iter()
            .map(|r| r.avg_proof_size_bytes)
            .fold(0.0f64, |a, b| a.max(b))
            * 1.2;

        let mut chart = ChartBuilder::on(&root)
            .caption("Avg Proof Size by Policy", ("sans-serif", 24))
            .margin(20)
            .x_label_area_size(60)
            .y_label_area_size(60)
            .build_cartesian_2d(0..policies.len(), 0.0..max_size)?;

        chart
            .configure_mesh()
            .y_desc("Avg Proof Size (bytes)")
            .x_desc("Policy")
            .draw()?;

        let colors = [&RED, &BLUE, &GREEN, &MAGENTA];

        for (i, policy) in policies.iter().enumerate() {
            let avg_size: f64 = results
                .iter()
                .filter(|r| &r.policy_name == policy)
                .map(|r| r.avg_proof_size_bytes)
                .sum::<f64>()
                / scenarios.len() as f64;

            chart.draw_series(std::iter::once(Rectangle::new(
                [(i, 0.0), (i + 1, avg_size)],
                colors[i % colors.len()].mix(0.7).filled(),
            )))?;
        }

        root.present()?;
    }

    Ok(())
}
