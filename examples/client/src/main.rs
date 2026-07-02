//! CatalystSVM Client
//!
//! Example client to interact with validator and verifier nodes
//!
//! Usage:
//!   # Submit a transaction
//!   cargo run -p catalyst-client -- submit --sender alice --program test --data "0102030405"
//!
//!   # Check validator status
//!   cargo run -p catalyst-client -- status
//!
//!   # Force seal current batch
//!   cargo run -p catalyst-client -- seal
//!
//!   # Verify a batch
//!   cargo run -p catalyst-client -- verify --batch-id batch_000001

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[derive(Parser, Debug)]
#[command(name = "catalyst-client")]
#[command(about = "CatalystSVM client - interact with validator/verifier nodes")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8899")]
    validator: String,

    #[arg(long, default_value = "127.0.0.1:8900")]
    verifier: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Submit {
        #[arg(long)]
        sender: String,

        #[arg(long)]
        program: String,

        #[arg(long, default_value = "")]
        data: String,

        #[arg(long, default_value = "normal")]
        priority: String,

        #[arg(long, default_value = "1")]
        count: usize,
    },

    Status,

    Seal,

    Proofs {
        #[arg(long, default_value = "10")]
        limit: usize,
    },

    Verify {
        #[arg(long)]
        batch_id: String,
    },

    VerifierStatus,

    Attestation {
        #[arg(long)]
        batch_id: String,
    },

    Benchmark {
        #[arg(long, default_value = "100")]
        tx_count: usize,

        #[arg(long, default_value = "10")]
        batch_size: usize,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct RpcRequest {
    method: String,
    params: serde_json::Value,
    id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct RpcResponse {
    result: serde_json::Value,
    id: u64,
    error: Option<String>,
}

async fn send_rpc(
    addr: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<RpcResponse, String> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    let req = RpcRequest {
        method: method.to_string(),
        params,
        id: 1,
    };

    let mut req_bytes = serde_json::to_vec(&req).unwrap();
    req_bytes.push(b'\n');

    stream
        .write_all(&req_bytes)
        .await
        .map_err(|e| format!("Write failed: {}", e))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("Read failed: {}", e))?;

    serde_json::from_str(&line).map_err(|e| format!("Parse failed: {}", e))
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    let hex = hex.trim_start_matches("0x");
    (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    match args.command {
        Command::Submit {
            sender,
            program,
            data,
            priority,
            count,
        } => {
            let instruction_data = if data.is_empty() {
                vec![3u8]
            } else {
                hex_to_bytes(&data)
            };

            println!("Submitting {} transaction(s)...", count);

            for i in 0..count {
                let params = serde_json::json!({
                    "sender": format!("{}_{}", sender, i),
                    "program_id": program,
                    "instruction_data": instruction_data,
                    "priority": priority,
                });

                match send_rpc(&args.validator, "submitTransaction", params).await {
                    Ok(resp) => {
                        if let Some(err) = resp.error {
                            eprintln!("Error: {}", err);
                        } else if let Some(tx_id) = resp.result.get("tx_id") {
                            println!("  Submitted: {}", tx_id);
                        }
                    }
                    Err(e) => eprintln!("Failed: {}", e),
                }
            }
        }

        Command::Status => {
            match send_rpc(&args.validator, "getStatus", serde_json::json!({})).await {
                Ok(resp) => {
                    if let Some(err) = resp.error {
                        eprintln!("Error: {}", err);
                    } else {
                        println!("Validator Status:");
                        println!("{}", serde_json::to_string_pretty(&resp.result)?);
                    }
                }
                Err(e) => eprintln!("Failed: {}", e),
            }
        }

        Command::Seal => {
            match send_rpc(&args.validator, "forceSeal", serde_json::json!({})).await {
                Ok(resp) => {
                    if let Some(err) = resp.error {
                        eprintln!("Error: {}", err);
                    } else if let Some(batch_id) = resp.result.get("batch_id") {
                        println!("Sealed batch: {}", batch_id);
                    }
                }
                Err(e) => eprintln!("Failed: {}", e),
            }
        }

        Command::Proofs { limit } => {
            match send_rpc(&args.validator, "getProofs", serde_json::json!({})).await {
                Ok(resp) => {
                    if let Some(err) = resp.error {
                        eprintln!("Error: {}", err);
                    } else {
                        let empty = vec![];
                        let proofs = resp.result.as_array().unwrap_or(&empty);
                        println!("Generated Proofs ({}):", proofs.len());
                        for proof in proofs.iter().take(limit) {
                            println!(
                                "  {} - {} txs, {}ms proving time, {} bytes",
                                proof.get("batch_id").unwrap_or(&serde_json::Value::Null),
                                proof.get("tx_count").unwrap_or(&serde_json::Value::Null),
                                proof
                                    .get("proving_time_ms")
                                    .unwrap_or(&serde_json::Value::Null),
                                proof
                                    .get("proof_size_bytes")
                                    .unwrap_or(&serde_json::Value::Null),
                            );
                        }
                    }
                }
                Err(e) => eprintln!("Failed: {}", e),
            }
        }

        Command::Verify { batch_id } => {
            let params = serde_json::json!({ "batch_id": batch_id });
            match send_rpc(&args.verifier, "verifyBatch", params).await {
                Ok(resp) => {
                    if let Some(err) = resp.error {
                        eprintln!("Verification failed: {}", err);
                    } else {
                        println!("Verification Result:");
                        println!("{}", serde_json::to_string_pretty(&resp.result)?);
                    }
                }
                Err(e) => eprintln!("Failed: {}", e),
            }
        }

        Command::VerifierStatus => {
            match send_rpc(&args.verifier, "getStatus", serde_json::json!({})).await {
                Ok(resp) => {
                    if let Some(err) = resp.error {
                        eprintln!("Error: {}", err);
                    } else {
                        println!("Verifier Status:");
                        println!("{}", serde_json::to_string_pretty(&resp.result)?);
                    }
                }
                Err(e) => eprintln!("Failed: {}", e),
            }
        }

        Command::Attestation { batch_id } => {
            let params = serde_json::json!({ "batch_id": batch_id });
            match send_rpc(&args.verifier, "getAttestation", params).await {
                Ok(resp) => {
                    if let Some(err) = resp.error {
                        eprintln!("Error: {}", err);
                    } else if let Some(att) = resp.result.get("attestation") {
                        println!("Attestation for {}: {}", batch_id, att);
                    }
                }
                Err(e) => eprintln!("Failed: {}", e),
            }
        }

        Command::Benchmark {
            tx_count,
            batch_size,
        } => {
            println!(
                "Running benchmark: {} txs, {} per batch",
                tx_count, batch_size
            );
            let start = std::time::Instant::now();

            for i in 0..tx_count {
                let params = serde_json::json!({
                    "sender": format!("bench_user_{}", i % 100),
                    "program_id": "benchmark_program",
                    "instruction_data": vec![3u8],
                    "priority": "normal",
                });

                if let Err(e) = send_rpc(&args.validator, "submitTransaction", params).await {
                    eprintln!("Submit failed: {}", e);
                }

                if (i + 1) % batch_size == 0 {
                    println!("  Submitted {} txs, sealing batch...", i + 1);
                    if let Err(e) =
                        send_rpc(&args.validator, "forceSeal", serde_json::json!({})).await
                    {
                        eprintln!("Seal failed: {}", e);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }

            if tx_count % batch_size != 0 {
                println!("  Sealing final batch...");
                if let Err(e) = send_rpc(&args.validator, "forceSeal", serde_json::json!({})).await
                {
                    eprintln!("Seal failed: {}", e);
                }
            }

            let elapsed = start.elapsed();
            println!("\nBenchmark complete:");
            println!("  Total txs: {}", tx_count);
            println!("  Time: {:.2}s", elapsed.as_secs_f64());
            println!("  TPS: {:.2}", tx_count as f64 / elapsed.as_secs_f64());

            match send_rpc(&args.validator, "getStatus", serde_json::json!({})).await {
                Ok(resp) => {
                    println!("\nValidator status:");
                    println!("{}", serde_json::to_string_pretty(&resp.result)?);
                }
                Err(e) => eprintln!("Status failed: {}", e),
            }
        }
    }

    Ok(())
}
