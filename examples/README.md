# CatalystSVM Examples

Production examples showing how to run CatalystSVM as a validator company or node operator.

## Architecture

```
┌─────────────────┐     transactions     ┌─────────────────┐
│   Users/dApps   │ ──────────────────▶  │    Validator    │
└─────────────────┘                      │     Node        │
                                         │                 │
                                         │  • Batch txs    │
                                         │  • Execute      │
                                         │  • Generate ZK  │
                                         │    proof (SP1)  │
                                         └────────┬────────┘
                                                  │
                                           proof  │
                                                  ▼
                                         ┌─────────────────┐
                                         │    Verifier     │
                                         │     Node        │
                                         │                 │
                                         │  • Verify proof │
                                         │  • Track state  │
                                         │  • Attestations │
                                         └─────────────────┘
```

## Quick Start

### 1. Start a Validator Node

```bash
cargo run --release -p catalyst-validator -- --rpc-port 8899 --proof-dir ./proofs

# Or using just:
just validator
```

Options:
- `--rpc-port`: Port for RPC server (default: 8899)
- `--proof-dir`: Directory to store proofs (default: ./proofs)
- `--policy`: Batching policy - fixed/time/hybrid/adaptive (default: hybrid)
- `--batch-size`: Max transactions per batch (default: 10)
- `--batch-timeout-ms`: Max wait time before sealing (default: 5000)
- `--broadcast-addr`: Address to broadcast proofs to (e.g., 127.0.0.1:8900)

### 2. Start a Verifier Node

```bash
cargo run --release -p catalyst-verifier-node -- --listen-port 8900 --proof-dir ./proofs --poll-proofs

# Or using just:
just verifier
```

Options:
- `--listen-port`: Port for incoming connections (default: 8900)
- `--proof-dir`: Directory to poll for proofs (default: ./proofs)
- `--poll-proofs`: Enable polling proof directory

### 3. Submit Transactions

```bash
# Submit a single transaction
cargo run -p catalyst-client -- submit --sender alice --program token_transfer --data "0102030405"

# Submit multiple transactions
cargo run -p catalyst-client -- submit --sender alice --program test --count 50

# Force seal current batch
cargo run -p catalyst-client -- seal

# Check validator status
cargo run -p catalyst-client -- status

# View generated proofs
cargo run -p catalyst-client -- proofs
```

### 4. Verify Proofs

```bash
# Verify a specific batch
cargo run -p catalyst-client -- verify --batch-id batch_000001

# Check verifier status
cargo run -p catalyst-client -- verifier-status

# Get attestation for a batch
cargo run -p catalyst-client -- attestation --batch-id batch_000001
```

### 5. Run a Benchmark

```bash
# Submit 100 transactions, 10 per batch
cargo run -p catalyst-client -- benchmark --tx-count 100 --batch-size 10
```

## Full Demo

```bash
# Terminal 1: Start validator
just validator

# Terminal 2: Start verifier  
just verifier

# Terminal 3: Submit transactions
just submit 5

# Wait for batch to seal (5 sec timeout) and proof generation (5-15 min)
# Watch Terminal 1 for "ZK proof generated" message

# Then verify:
cargo run -p catalyst-client -- verify --batch-id batch_000001
```

Note: ZK proof generation takes 5-15 minutes per batch on CPU. This is real
cryptographic proof generation using SP1 (Succinct's RISC-V zkVM).

## Production Deployment

### Validator Node Requirements
- 8+ CPU cores (SP1 proving is CPU-intensive)
- 32GB+ RAM
- SSD storage for proofs
- Stable network connection

### Verifier Node Requirements
- 4+ CPU cores
- 16GB+ RAM
- Network connectivity to validators

### Scaling
- Run multiple validators behind a load balancer
- Run multiple verifiers for redundancy
- Use Succinct prover network for faster proving at scale

### Example Production Config

```bash
# Validator with adaptive batching, broadcasting to verifiers
cargo run --release -p catalyst-validator -- \
  --policy adaptive \
  --batch-size 100 \
  --batch-timeout-ms 30000 \
  --proof-dir /data/proofs \
  --broadcast-addr verifier1.example.com:8900

# Verifier polling proof directory
cargo run --release -p catalyst-verifier-node -- \
  --proof-dir /data/proofs \
  --poll-proofs \
  --poll-interval-ms 1000
```

## RPC API

### Validator RPC Methods

| Method | Params | Description |
|--------|--------|-------------|
| `submitTransaction` | `{sender, program_id, instruction_data, priority}` | Submit a transaction |
| `getStatus` | `{}` | Get validator status |
| `getProofs` | `{}` | List generated proofs |
| `forceSeal` | `{}` | Force seal current batch |

### Verifier RPC Methods

| Method | Params | Description |
|--------|--------|-------------|
| `verifyBatch` | `{batch_id}` | Verify a proof |
| `getStatus` | `{}` | Get verifier status |
| `getVerifiedBatches` | `{limit}` | List verified batches |
| `getAttestation` | `{batch_id}` | Get attestation for a batch |

## Integration with Your Application

```rust
use tokio::net::TcpStream;
use serde_json::json;

async fn submit_transaction(validator: &str, tx: Transaction) -> Result<String, Error> {
    let mut stream = TcpStream::connect(validator).await?;
    
    let request = json!({
        "method": "submitTransaction",
        "params": {
            "sender": tx.sender,
            "program_id": tx.program_id,
            "instruction_data": tx.data,
            "priority": "normal"
        },
        "id": 1
    });
    
    // Send request, read response...
}
```

## Monitoring

Both nodes emit structured logs via `tracing`. Set log level:

```bash
RUST_LOG=info cargo run -p catalyst-validator -- ...
RUST_LOG=debug cargo run -p catalyst-verifier-node -- ...
```

Key log events:
- `Transaction submitted` - New tx in queue
- `Batch sealed` - Batch ready for proving
- `Batch executed` - Execution complete
- `Proof generated` - SP1 proof ready
- `Proof verified successfully` - Verification passed
