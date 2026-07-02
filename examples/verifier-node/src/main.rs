//! CatalystSVM Verifier Node
//!
//! A verifier node that:
//! 1. Receives proofs from validators (via broadcast or file polling)
//! 2. Verifies SP1 proofs cryptographically
//! 3. Maintains a verified state root chain
//! 4. Provides attestation for verified batches
//!
//! Usage:
//!   cargo run --release -p catalyst-verifier-node -- --listen-port 8900 --proof-dir ./proofs

use catalyst_common::{
    Clock, ExecutionTrace, Proof, ProofCommitment, SystemClock, Verifier as CatalystVerifier,
};
use catalyst_sp1_verifier::Sp1Verifier;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "catalyst-verifier-node")]
#[command(about = "CatalystSVM verifier node - verifies ZK proofs")]
struct Args {
    #[arg(long, default_value = "8900")]
    listen_port: u16,

    #[arg(long, default_value = "./proofs")]
    proof_dir: PathBuf,

    #[arg(long)]
    poll_proofs: bool,

    #[arg(long, default_value = "5000")]
    poll_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedBatch {
    pub batch_id: String,
    pub proof_hash: String,
    pub state_root_pre: String,
    pub state_root_post: String,
    pub tx_count: usize,
    pub verification_time_ms: u64,
    pub verified_at: u64,
    pub attestation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub batch_id: String,
    pub proof_hash: String,
    pub commitment: ProofCommitment,
    pub proof_size: usize,
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

struct VerifierState {
    verified_batches: Vec<VerifiedBatch>,
    pending_verifications: HashMap<String, ProofMessage>,
    current_state_root: String,
    verification_count: u64,
    failed_count: u64,
}

impl Default for VerifierState {
    fn default() -> Self {
        Self {
            verified_batches: Vec::new(),
            pending_verifications: HashMap::new(),
            current_state_root: "genesis".to_string(),
            verification_count: 0,
            failed_count: 0,
        }
    }
}

struct VerifierNode {
    state: Arc<RwLock<VerifierState>>,
    verifier: Arc<Sp1Verifier>,
    clock: Arc<SystemClock>,
    proof_dir: PathBuf,
    node_id: String,
}

impl VerifierNode {
    fn new(args: &Args) -> Self {
        info!("Initializing SP1 verifier...");

        let verifier = Arc::new(Sp1Verifier::new());
        let node_id = format!("verifier_{:08x}", rand_u32());

        info!("SP1 verifier initialized");

        Self {
            state: Arc::new(RwLock::new(VerifierState::default())),
            verifier,
            clock: Arc::new(SystemClock),
            proof_dir: args.proof_dir.clone(),
            node_id,
        }
    }

    async fn verify_proof_file(&self, batch_id: &str) -> Result<VerifiedBatch, String> {
        let proof_path = self.proof_dir.join(format!("{}.proof", batch_id));
        let trace_path = self.proof_dir.join(format!("{}.trace", batch_id));

        let proof_bytes = tokio::fs::read(&proof_path)
            .await
            .map_err(|e| format!("Failed to read proof: {}", e))?;

        let trace_bytes = tokio::fs::read(&trace_path)
            .await
            .map_err(|e| format!("Failed to read trace: {}", e))?;

        let proof: Proof =
            bincode::deserialize(&proof_bytes).map_err(|e| format!("Invalid proof: {}", e))?;

        let trace: ExecutionTrace =
            serde_json::from_slice(&trace_bytes).map_err(|e| format!("Invalid trace: {}", e))?;

        self.verify_proof(&proof, &trace).await
    }

    async fn verify_proof(
        &self,
        proof: &Proof,
        trace: &ExecutionTrace,
    ) -> Result<VerifiedBatch, String> {
        info!(
            batch_id = %proof.batch_id,
            proof_size_bytes = proof.proof_size_bytes,
            "Starting cryptographic proof verification..."
        );

        let start = std::time::Instant::now();

        let verifier = self.verifier.clone();
        let clock = self.clock.clone();
        let proof_clone = proof.clone();
        let trace_clone = trace.clone();

        let result = tokio::task::spawn_blocking(move || {
            verifier.verify(&proof_clone, &trace_clone, clock.as_ref())
        })
        .await
        .map_err(|e| format!("Verification task failed: {}", e))?
        .map_err(|e| format!("Verification error: {}", e))?;

        let verification_time_ms = start.elapsed().as_millis() as u64;

        if !result.is_valid {
            let mut state = self.state.write().await;
            state.failed_count += 1;
            return Err(format!(
                "Verification failed: {}",
                result.error.unwrap_or_default()
            ));
        }

        let attestation = format!(
            "ATTESTED:{}:{}:{}:{}",
            self.node_id,
            proof.batch_id,
            proof.proof_hash,
            self.clock.now_ms()
        );
        let attestation_hash = catalyst_common::hash_str(&attestation);

        let verified = VerifiedBatch {
            batch_id: proof.batch_id.clone(),
            proof_hash: proof.proof_hash.clone(),
            state_root_pre: proof.commitment.state_root_pre.clone(),
            state_root_post: proof.commitment.state_root_post.clone(),
            tx_count: proof.commitment.tx_count,
            verification_time_ms,
            verified_at: self.clock.now_ms(),
            attestation: attestation_hash,
        };

        {
            let mut state = self.state.write().await;
            if state.current_state_root != "genesis"
                && state.current_state_root != verified.state_root_pre
            {
                warn!(
                    expected = %state.current_state_root,
                    got = %verified.state_root_pre,
                    "State root mismatch - gap in verified chain"
                );
            }
            state.current_state_root = verified.state_root_post.clone();
            state.verification_count += 1;
            state.verified_batches.push(verified.clone());
        }

        info!(
            batch_id = %proof.batch_id,
            verification_time_ms = verification_time_ms,
            tx_count = proof.commitment.tx_count,
            state_root = %verified.state_root_post,
            "Proof verified successfully"
        );

        Ok(verified)
    }

    async fn handle_proof_broadcast(&self, msg: ProofMessage) -> Result<(), String> {
        info!(
            batch_id = %msg.batch_id,
            proof_hash = %msg.proof_hash,
            "Received proof broadcast"
        );

        {
            let mut state = self.state.write().await;
            state
                .pending_verifications
                .insert(msg.batch_id.clone(), msg.clone());
        }

        match self.verify_proof_file(&msg.batch_id).await {
            Ok(_verified) => {
                let mut state = self.state.write().await;
                state.pending_verifications.remove(&msg.batch_id);
                Ok(())
            }
            Err(e) => {
                warn!(batch_id = %msg.batch_id, error = %e, "Verification failed");
                Err(e)
            }
        }
    }

    async fn handle_rpc(&self, req: RpcRequest) -> RpcResponse {
        match req.method.as_str() {
            "verifyBatch" => {
                let batch_id = req.params.get("batch_id").and_then(|v| v.as_str());
                match batch_id {
                    Some(id) => match self.verify_proof_file(id).await {
                        Ok(verified) => RpcResponse {
                            result: serde_json::to_value(&verified).unwrap(),
                            id: req.id,
                            error: None,
                        },
                        Err(e) => RpcResponse {
                            result: serde_json::Value::Null,
                            id: req.id,
                            error: Some(e),
                        },
                    },
                    None => RpcResponse {
                        result: serde_json::Value::Null,
                        id: req.id,
                        error: Some("Missing batch_id".into()),
                    },
                }
            }

            "getStatus" => {
                let state = self.state.read().await;
                RpcResponse {
                    result: serde_json::json!({
                        "node_id": self.node_id,
                        "verified_count": state.verification_count,
                        "failed_count": state.failed_count,
                        "pending_count": state.pending_verifications.len(),
                        "current_state_root": state.current_state_root,
                    }),
                    id: req.id,
                    error: None,
                }
            }

            "getVerifiedBatches" => {
                let state = self.state.read().await;
                let limit = req
                    .params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100) as usize;
                let batches: Vec<_> = state
                    .verified_batches
                    .iter()
                    .rev()
                    .take(limit)
                    .cloned()
                    .collect();
                RpcResponse {
                    result: serde_json::to_value(&batches).unwrap(),
                    id: req.id,
                    error: None,
                }
            }

            "getAttestation" => {
                let batch_id = req.params.get("batch_id").and_then(|v| v.as_str());
                match batch_id {
                    Some(id) => {
                        let state = self.state.read().await;
                        let attestation = state
                            .verified_batches
                            .iter()
                            .find(|b| b.batch_id == id)
                            .map(|b| &b.attestation);
                        match attestation {
                            Some(a) => RpcResponse {
                                result: serde_json::json!({ "attestation": a }),
                                id: req.id,
                                error: None,
                            },
                            None => RpcResponse {
                                result: serde_json::Value::Null,
                                id: req.id,
                                error: Some("Batch not verified".into()),
                            },
                        }
                    }
                    None => RpcResponse {
                        result: serde_json::Value::Null,
                        id: req.id,
                        error: Some("Missing batch_id".into()),
                    },
                }
            }

            _ => RpcResponse {
                result: serde_json::Value::Null,
                id: req.id,
                error: Some(format!("Unknown method: {}", req.method)),
            },
        }
    }

    async fn poll_proof_directory(&self) {
        let mut processed_set: std::collections::HashSet<String> = std::collections::HashSet::new();

        loop {
            if let Ok(mut entries) = tokio::fs::read_dir(&self.proof_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if path.extension().map(|e| e == "proof").unwrap_or(false) {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            let batch_id = stem.to_string();
                            if !processed_set.contains(&batch_id) {
                                match self.verify_proof_file(&batch_id).await {
                                    Ok(_) => {
                                        processed_set.insert(batch_id);
                                    }
                                    Err(e) => {
                                        warn!(batch_id = %batch_id, error = %e, "Failed to verify");
                                        processed_set.insert(batch_id);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(5000)).await;
        }
    }
}

fn rand_u32() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    nanos ^ (std::process::id() << 16)
}

async fn handle_connection(verifier: Arc<VerifierNode>, stream: TcpStream) {
    let peer = stream.peer_addr().ok();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                if let Ok(msg) = serde_json::from_str::<ProofMessage>(&line) {
                    if msg.msg_type == "proof" {
                        if let Err(e) = verifier.handle_proof_broadcast(msg).await {
                            warn!(error = %e, "Failed to handle proof broadcast");
                        }
                        continue;
                    }
                }

                let req: Result<RpcRequest, _> = serde_json::from_str(&line);
                let response = match req {
                    Ok(r) => verifier.handle_rpc(r).await,
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
                .add_directive("catalyst_verifier_node=info".parse()?)
                .add_directive("catalyst_sp1_verifier=info".parse()?),
        )
        .init();

    let args = Args::parse();

    info!(
        port = args.listen_port,
        proof_dir = %args.proof_dir.display(),
        poll = args.poll_proofs,
        "Starting CatalystSVM Verifier Node"
    );

    // Initialize verifier (including SP1) BEFORE entering tokio runtime
    let verifier = Arc::new(VerifierNode::new(&args));

    // Now start the async runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_verifier(verifier, args))
}

async fn run_verifier(
    verifier: Arc<VerifierNode>,
    args: Args,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.poll_proofs {
        let poll_verifier = verifier.clone();
        tokio::spawn(async move {
            poll_verifier.poll_proof_directory().await;
        });
    }

    let listener = TcpListener::bind(format!("0.0.0.0:{}", args.listen_port)).await?;
    info!(port = args.listen_port, "Verifier node listening");

    loop {
        let (stream, addr) = listener.accept().await?;
        info!(peer = %addr, "Client connected");
        let v = verifier.clone();
        tokio::spawn(handle_connection(v, stream));
    }
}
