//! CatalystSVM Validator Node
//!
//! A validator node that:
//! 1. Receives transactions from users/RPCs
//! 2. Batches them according to policy
//! 3. Executes batches deterministically
//! 4. Generates SP1 ZK proofs
//! 5. Broadcasts proofs to verifier nodes
//!
//! Usage:
//!   cargo run --release -p catalyst-validator -- --rpc-port 8899 --proof-dir ./proofs

use catalyst_batch_policy::{AdaptivePolicy, FixedCountPolicy, FixedTimePolicy, HybridPolicy};
use catalyst_common::{
    Batch, BatchStatus, Clock, ExecutionTrace, PolicyEngine, Proof, Prover, SealReason,
    SystemClock, Transaction, TransactionId, TransactionPriority,
};
use catalyst_execution_engine::FullExecutor;
use catalyst_sp1_prover::Sp1Prover;
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "catalyst-validator")]
#[command(about = "CatalystSVM validator node - produces ZK proofs for batches")]
struct Args {
    #[arg(long, default_value = "8899")]
    rpc_port: u16,

    #[arg(long, default_value = "./proofs")]
    proof_dir: PathBuf,

    #[arg(long, default_value = "hybrid")]
    policy: PolicyType,

    #[arg(long, default_value = "10")]
    batch_size: usize,

    #[arg(long, default_value = "5000")]
    batch_timeout_ms: u64,

    #[arg(long)]
    broadcast_addr: Option<String>,
}

#[derive(Clone, Debug, ValueEnum)]
enum PolicyType {
    Fixed,
    Time,
    Hybrid,
    Adaptive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub method: String,
    pub params: serde_json::Value,
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub result: serde_json::Value,
    pub id: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTxParams {
    pub sender: String,
    pub program_id: String,
    pub instruction_data: Vec<u8>,
    pub priority: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofRecord {
    pub batch_id: String,
    pub proof_hash: String,
    pub state_root_pre: String,
    pub state_root_post: String,
    pub tx_count: usize,
    pub proving_time_ms: u64,
    pub proof_size_bytes: usize,
    pub timestamp: u64,
}

struct ValidatorState {
    pending_txs: VecDeque<Transaction>,
    batches: Vec<Batch>,
    proofs: Vec<ProofRecord>,
    #[allow(dead_code)]
    state: BTreeMap<String, u64>,
    tx_counter: u64,
    batch_counter: u64,
}

impl Default for ValidatorState {
    fn default() -> Self {
        let mut state = BTreeMap::new();
        for i in 0..100 {
            state.insert(format!("account_{}", i), 1_000_000);
        }
        Self {
            pending_txs: VecDeque::new(),
            batches: Vec::new(),
            proofs: Vec::new(),
            state,
            tx_counter: 0,
            batch_counter: 0,
        }
    }
}

struct Validator {
    state: Arc<RwLock<ValidatorState>>,
    policy: Arc<dyn PolicyEngine>,
    executor: Arc<FullExecutor>,
    prover: Arc<Sp1Prover>,
    clock: Arc<SystemClock>,
    proof_dir: PathBuf,
    batch_size: usize,
    broadcast_addr: Option<String>,
}

impl Validator {
    fn new(args: &Args) -> Self {
        info!("Initializing SP1 prover (this may take a moment on first run)...");

        let policy: Arc<dyn PolicyEngine> = match args.policy {
            PolicyType::Fixed => Arc::new(FixedCountPolicy::new(args.batch_size)),
            PolicyType::Time => Arc::new(FixedTimePolicy::new(args.batch_timeout_ms)),
            PolicyType::Hybrid => {
                Arc::new(HybridPolicy::new(args.batch_size, args.batch_timeout_ms))
            }
            PolicyType::Adaptive => Arc::new(AdaptivePolicy::new(
                args.batch_size,
                200,
                args.batch_timeout_ms,
                100,
            )),
        };

        let prover = Arc::new(Sp1Prover::new());

        std::fs::create_dir_all(&args.proof_dir).ok();

        info!("SP1 prover initialized");

        Self {
            state: Arc::new(RwLock::new(ValidatorState::default())),
            policy,
            executor: Arc::new(FullExecutor::new(200_000)),
            prover,
            clock: Arc::new(SystemClock),
            proof_dir: args.proof_dir.clone(),
            batch_size: args.batch_size,
            broadcast_addr: args.broadcast_addr.clone(),
        }
    }

    async fn submit_transaction(&self, params: SubmitTxParams) -> Result<String, String> {
        let mut state = self.state.write().await;
        state.tx_counter += 1;

        let tx_id = format!("tx_{:08x}", state.tx_counter);
        let priority = match params.priority.as_deref() {
            Some("low") => TransactionPriority::Low,
            Some("high") => TransactionPriority::High,
            Some("critical") => TransactionPriority::Critical,
            _ => TransactionPriority::Normal,
        };

        let mut tx = Transaction::new(TransactionId::new(&tx_id), params.sender, params.program_id);
        tx.instruction_data = params.instruction_data;
        tx.priority = priority;
        tx.arrival_ts = self.clock.now_ms();

        state.pending_txs.push_back(tx);
        info!(tx_id = %tx_id, queue_len = state.pending_txs.len(), "Transaction submitted");

        Ok(tx_id)
    }

    async fn check_and_seal_batch(&self) -> Option<String> {
        let mut state = self.state.write().await;

        if state.pending_txs.is_empty() {
            return None;
        }

        let queue_len = state.pending_txs.len();
        let oldest_wait_ms = if let Some(oldest) = state.pending_txs.front() {
            self.clock.now_ms().saturating_sub(oldest.arrival_ts)
        } else {
            0
        };

        if !self.policy.should_seal(queue_len, oldest_wait_ms, 0.0) {
            return None;
        }

        state.batch_counter += 1;
        let batch_id = format!("batch_{:06}", state.batch_counter);

        let txs_to_process: Vec<Transaction> = state
            .pending_txs
            .drain(..queue_len.min(self.batch_size))
            .collect();

        let mut batch = Batch::new(&batch_id, self.clock.now_ms());
        batch.tx_ids = txs_to_process.iter().map(|tx| tx.tx_id.clone()).collect();
        batch.seal_ts = self.clock.now_ms();
        batch.reason_sealed = SealReason::Adaptive;
        batch.status = BatchStatus::Sealed;

        info!(
            batch_id = %batch_id,
            tx_count = txs_to_process.len(),
            "Batch sealed"
        );

        drop(state);

        if let Err(e) = self.process_batch(&batch_id, txs_to_process).await {
            error!(batch_id = %batch_id, error = %e, "Failed to process batch");
            return None;
        }

        Some(batch_id)
    }

    async fn process_batch(
        &self,
        batch_id: &str,
        transactions: Vec<Transaction>,
    ) -> Result<(), String> {
        let start = Instant::now();

        let batch = {
            let mut batch = Batch::new(batch_id, self.clock.now_ms());
            batch.tx_ids = transactions.iter().map(|tx| tx.tx_id.clone()).collect();
            batch.seal_ts = self.clock.now_ms();
            batch.status = BatchStatus::Sealed;
            batch
        };

        let trace = self
            .executor
            .execute_with_transactions(&batch, &transactions, self.clock.as_ref())
            .map_err(|e| format!("Execution failed: {}", e))?;

        info!(
            batch_id = %batch_id,
            tx_count = trace.entries.len(),
            success = trace.success_count,
            failed = trace.failure_count,
            compute = trace.total_compute,
            "Batch executed"
        );

        info!(batch_id = %batch_id, "Starting ZK proof generation (this takes several minutes)...");

        let prover = self.prover.clone();
        let clock = self.clock.clone();
        let trace_clone = trace.clone();
        let transactions_clone = transactions.clone();

        let proof = tokio::task::spawn_blocking(move || {
            prover.prove(&trace_clone, &transactions_clone, clock.as_ref())
        })
        .await
        .map_err(|e| format!("Proving task failed: {}", e))?
        .map_err(|e| format!("Proving failed: {}", e))?;

        let elapsed = start.elapsed();

        info!(
            batch_id = %batch_id,
            proving_time_ms = proof.proving_time_ms,
            proof_size_bytes = proof.proof_size_bytes,
            elapsed_sec = elapsed.as_secs(),
            "ZK proof generated"
        );

        let proof_record = ProofRecord {
            batch_id: batch_id.to_string(),
            proof_hash: proof.proof_hash.clone(),
            state_root_pre: proof.commitment.state_root_pre.clone(),
            state_root_post: proof.commitment.state_root_post.clone(),
            tx_count: proof.commitment.tx_count,
            proving_time_ms: proof.proving_time_ms,
            proof_size_bytes: proof.proof_size_bytes,
            timestamp: self.clock.now_ms(),
        };

        self.save_proof(&proof, &trace).await?;

        {
            let mut state = self.state.write().await;
            state.proofs.push(proof_record.clone());
        }

        if let Some(addr) = &self.broadcast_addr {
            self.broadcast_proof(addr, &proof).await;
        }

        Ok(())
    }

    async fn save_proof(&self, proof: &Proof, trace: &ExecutionTrace) -> Result<(), String> {
        let proof_path = self.proof_dir.join(format!("{}.proof", proof.batch_id));
        let trace_path = self.proof_dir.join(format!("{}.trace", proof.batch_id));

        let proof_bytes = bincode::serialize(proof).map_err(|e| e.to_string())?;
        let trace_bytes = serde_json::to_vec_pretty(trace).map_err(|e| e.to_string())?;

        tokio::fs::write(&proof_path, proof_bytes)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(&trace_path, trace_bytes)
            .await
            .map_err(|e| e.to_string())?;

        info!(
            proof_path = %proof_path.display(),
            trace_path = %trace_path.display(),
            "Proof and trace saved"
        );

        Ok(())
    }

    async fn broadcast_proof(&self, addr: &str, proof: &Proof) {
        match TcpStream::connect(addr).await {
            Ok(mut stream) => {
                let msg = serde_json::json!({
                    "type": "proof",
                    "batch_id": proof.batch_id,
                    "proof_hash": proof.proof_hash,
                    "commitment": proof.commitment,
                    "proof_size": proof.proof_size_bytes,
                });
                let data = serde_json::to_vec(&msg).unwrap();
                if let Err(e) = stream.write_all(&data).await {
                    warn!(error = %e, "Failed to broadcast proof");
                } else {
                    info!(addr = %addr, batch_id = %proof.batch_id, "Proof broadcast sent");
                }
            }
            Err(e) => {
                warn!(addr = %addr, error = %e, "Failed to connect to verifier");
            }
        }
    }

    async fn handle_rpc(&self, req: RpcRequest) -> RpcResponse {
        match req.method.as_str() {
            "submitTransaction" => {
                let params: Result<SubmitTxParams, _> = serde_json::from_value(req.params);
                match params {
                    Ok(p) => match self.submit_transaction(p).await {
                        Ok(tx_id) => RpcResponse {
                            result: serde_json::json!({ "tx_id": tx_id }),
                            id: req.id,
                            error: None,
                        },
                        Err(e) => RpcResponse {
                            result: serde_json::Value::Null,
                            id: req.id,
                            error: Some(e),
                        },
                    },
                    Err(e) => RpcResponse {
                        result: serde_json::Value::Null,
                        id: req.id,
                        error: Some(format!("Invalid params: {}", e)),
                    },
                }
            }

            "getStatus" => {
                let state = self.state.read().await;
                RpcResponse {
                    result: serde_json::json!({
                        "pending_txs": state.pending_txs.len(),
                        "total_batches": state.batches.len(),
                        "total_proofs": state.proofs.len(),
                        "policy": self.policy.name(),
                    }),
                    id: req.id,
                    error: None,
                }
            }

            "getProofs" => {
                let state = self.state.read().await;
                RpcResponse {
                    result: serde_json::to_value(&state.proofs).unwrap(),
                    id: req.id,
                    error: None,
                }
            }

            "forceSeal" => match self.check_and_seal_batch().await {
                Some(batch_id) => RpcResponse {
                    result: serde_json::json!({ "batch_id": batch_id }),
                    id: req.id,
                    error: None,
                },
                None => RpcResponse {
                    result: serde_json::Value::Null,
                    id: req.id,
                    error: Some("No transactions to seal".into()),
                },
            },

            _ => RpcResponse {
                result: serde_json::Value::Null,
                id: req.id,
                error: Some(format!("Unknown method: {}", req.method)),
            },
        }
    }
}

async fn handle_connection(validator: Arc<Validator>, stream: TcpStream) {
    let peer = stream.peer_addr().ok();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let req: Result<RpcRequest, _> = serde_json::from_str(&line);
                let response = match req {
                    Ok(r) => validator.handle_rpc(r).await,
                    Err(e) => RpcResponse {
                        result: serde_json::Value::Null,
                        id: 0,
                        error: Some(format!("Parse error: {}", e)),
                    },
                };

                let mut resp_bytes = serde_json::to_vec(&response).unwrap();
                resp_bytes.push(b'\n');
                if writer.write_all(&resp_bytes).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    if let Some(addr) = peer {
        info!(peer = %addr, "Client disconnected");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("catalyst_validator=info".parse()?)
                .add_directive("catalyst_sp1_prover=info".parse()?),
        )
        .init();

    let args = Args::parse();

    info!(
        port = args.rpc_port,
        policy = ?args.policy,
        batch_size = args.batch_size,
        proof_dir = %args.proof_dir.display(),
        "Starting CatalystSVM Validator"
    );

    // Initialize validator (including SP1 prover) BEFORE entering tokio runtime
    let validator = Arc::new(Validator::new(&args));

    // Now start the async runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_validator(validator, args))
}

async fn run_validator(
    validator: Arc<Validator>,
    args: Args,
) -> Result<(), Box<dyn std::error::Error>> {
    let batch_validator = validator.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
        loop {
            interval.tick().await;
            batch_validator.check_and_seal_batch().await;
        }
    });

    let listener = TcpListener::bind(format!("0.0.0.0:{}", args.rpc_port)).await?;
    info!(port = args.rpc_port, "RPC server listening");

    loop {
        let (stream, addr) = listener.accept().await?;
        info!(peer = %addr, "Client connected");
        let v = validator.clone();
        tokio::spawn(handle_connection(v, stream));
    }
}
