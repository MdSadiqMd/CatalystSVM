# CatalystSVM - Latency-Aware zkSVM Batcher
set shell := ["bash", "-c"]

default:
    just --list

# Build all crates
build:
    cargo build --workspace

# Build release
release:
    cargo build --workspace --release

# Run all tests
test:
    cargo test --workspace

# Clippy lint check
lint:
    cargo clippy --workspace -- -D warnings

# Format code
fmt:
    cargo fmt --all

# Format check (no changes)
fmt-check:
    cargo fmt --all -- --check

# Clean build artifacts
clean:
    cargo clean

# Run CLI simulate with defaults
simulate policy="hybrid" scenario="steady" seed="42" tx="500":
    cargo run -p catalyst-benchmark -- simulate --policy {{policy}} --scenario {{scenario}} --seed {{seed}} --tx-count {{tx}}

# Run full policy comparison
compare seed="42" tx="500" out="out":
    mkdir -p {{out}}
    cargo run -p catalyst-benchmark -- compare --seed {{seed}} --tx-count {{tx}} --out {{out}}

# Generate charts from results
chart input="out/results.json" out="out":
    mkdir -p {{out}}
    cargo run -p catalyst-benchmark -- chart --input {{input}} --out {{out}}

# Full benchmark: compare + charts
benchmark seed="42" tx="1000" out="out":
    mkdir -p {{out}}
    cargo run -p catalyst-benchmark -- compare --seed {{seed}} --tx-count {{tx}} --out {{out}}
    cargo run -p catalyst-benchmark -- chart --input {{out}}/results.json --out {{out}}
    @echo "Results in {{out}}/: results.json, results.csv, *.png"

# Build Anchor program
anchor-build:
    cd onchain/catalyst_batcher && anchor build

# Test Anchor program (requires local validator)
anchor-test:
    cd onchain/catalyst_batcher && anchor test

# Dev: watch + rebuild on changes
watch:
    cargo watch -c -x "build --workspace" -x "test --workspace"

# Show available scenarios
scenarios:
    @echo "steady, bursty, poisson, mixed_priority, idle_then_spike"

# Show available policies
policies:
    @echo "fixedcount, fixedtime, hybrid, adaptive"

# Quick sanity check
check: fmt-check lint test
    @echo "All checks passed"

# CI pipeline
ci: fmt-check lint test
    cargo doc --workspace --no-deps

# Build SP1 guest program (requires sp1up toolchain)
sp1-build:
    cd crates/sp1-program && cargo prove build
    mkdir -p crates/sp1-program/elf
    cp crates/sp1-program/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/catalyst-sp1-program crates/sp1-program/elf/riscv32im-succinct-zkvm-elf

# Build SP1 program with Docker for reproducibility
sp1-build-docker:
    cd crates/sp1-program && cargo prove build --docker
    mkdir -p crates/sp1-program/elf
    cp crates/sp1-program/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/catalyst-sp1-program crates/sp1-program/elf/riscv32im-succinct-zkvm-elf

# Run SP1-specific tests (slow, ~minutes per proof)
test-sp1:
    cargo test -p catalyst-sp1-prover -p catalyst-sp1-verifier -- --ignored

# Run all tests including slow SP1 tests
test-all: test test-sp1

# Start validator node (SP1 ZK prover) - clears old proofs first
# policy: hybrid (default), fixedcount, fixedtime, adaptive
validator policy="hybrid" port="8899":
    rm -rf ./proofs && mkdir -p ./proofs
    cargo run --release -p catalyst-validator -- --policy {{policy}} --rpc-port {{port}} --proof-dir ./proofs

# Start verifier node (SP1 ZK verifier)
verifier port="8900":
    cargo run --release -p catalyst-verifier-node -- --listen-port {{port}} --proof-dir ./proofs --poll-proofs

# Submit test transactions
submit count="10":
    cargo run -p catalyst-client -- submit --sender test_user --program test --count {{count}}

# Submit and wait for hybrid auto-seal (validator: --policy hybrid)
submit-hybrid count="10" wait="15":
    cargo run -p catalyst-client -- submit --sender test_user --program test --count {{count}}
    @echo "Waiting {{wait}}s for hybrid auto-seal..."
    sleep {{wait}}
    cargo run -p catalyst-client -- status

# Submit and wait for fixedcount auto-seal (validator: --policy fixedcount)
submit-fixedcount count="10" wait="15":
    cargo run -p catalyst-client -- submit --sender test_user --program test --count {{count}}
    @echo "Waiting {{wait}}s for fixedcount auto-seal..."
    sleep {{wait}}
    cargo run -p catalyst-client -- status

# Submit and wait for fixedtime auto-seal (validator: --policy fixedtime)
submit-fixedtime count="10" wait="15":
    cargo run -p catalyst-client -- submit --sender test_user --program test --count {{count}}
    @echo "Waiting {{wait}}s for fixedtime auto-seal..."
    sleep {{wait}}
    cargo run -p catalyst-client -- status

# Submit and wait for adaptive auto-seal (validator: --policy adaptive)
submit-adaptive count="10" wait="15":
    cargo run -p catalyst-client -- submit --sender test_user --program test --count {{count}}
    @echo "Waiting {{wait}}s for adaptive auto-seal..."
    sleep {{wait}}
    cargo run -p catalyst-client -- status

# Test hybrid policy: 5 txs, seals on count OR after 5s timeout
test-hybrid:
    cargo run -p catalyst-client -- submit --sender test_user --program test --count 5

# Test fixed-count policy: sends exactly at threshold (10 txs seals immediately)
test-fixed:
    cargo run -p catalyst-client -- submit --sender test_user --program test --count 10

# Test fixed-time policy: sends 3 txs, waits for timer to fire (requires --policy fixedtime validator)
test-time:
    cargo run -p catalyst-client -- submit --sender test_user --program test --count 3

# Test adaptive policy: burst of 50 txs to trigger high-load mode
test-adaptive:
    cargo run -p catalyst-client -- submit --sender test_user --program test --count 50

# Force seal and check status
seal:
    cargo run -p catalyst-client -- seal
    cargo run -p catalyst-client -- status

# Run demo
demo:
    @echo "Run in separate terminals:"
    @echo "  Terminal 1: just validator"
    @echo "  Terminal 2: just verifier"
    @echo "  Terminal 3: just submit 5"
    @echo ""
    @echo "Note: Each batch takes 5-15 minutes to generate ZK proof"
    @echo "      Watch Terminal 1 for 'ZK proof generated' message"
