# Katana TEE Client

A Rust client for interacting with Katana TEE attestations and generating SP1 proofs.

## Overview

This crate provides:
- **RPC Client**: Fetch TEE attestations from Katana nodes
- **Proof Generation**: Generate SP1 Groth16 proofs from attestations
- **CLI**: Command-line tool for the complete workflow

This crate uses [`amd_tee_registry_client`](../amd_tee_registry_client) for the underlying AMD attestation proving.

## Quick Start

### Library Usage

```rust
use katana_tee_client::{KatanaRpcClient, generate_sp1_proof};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Fetch attestation from Katana RPC
    let client = KatanaRpcClient::new("http://localhost:5050");
    let attestation = client.generate_quote().await?;

    println!("Block: {}", attestation.block_number);
    println!("State Root: {}", attestation.state_root);

    // Generate SP1 proof (uses amd_tee_registry_client)
    let proof = generate_sp1_proof(attestation).await?;
    println!("Verifier ID: {}", proof.program_id.verifier_id);

    Ok(())
}
```

### CLI Usage

```bash
# Build the CLI
cargo build -p katana_tee_client --release

# Fetch attestation
katana-tee fetch --rpc http://localhost:5050

# Execute SP1 program (mock mode, fast)
katana-tee execute --rpc http://localhost:5050

# Generate Groth16 proof via network
katana-tee prove --rpc http://localhost:5050 --prover network

# Show proof info
katana-tee info proof_output.json
```

## Commands

| Command | Description |
|---------|-------------|
| `fetch` | Fetch TEE attestation from Katana RPC |
| `execute` | Execute SP1 program in mock mode |
| `prove` | Generate SP1 Groth16 proof |
| `info` | Display proof file details |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `KATANA_RPC_URL` | Default Katana RPC endpoint |
| `SP1_PROVER` | Prover mode: `mock`, `cpu`, or `network` |
| `NETWORK_PRIVATE_KEY` | Private key for SP1 Prover Network |

## Examples

### Fetch Attestation

```bash
cargo run --example fetch_attestation -p katana_tee_client -- --rpc http://localhost:5050
```

### Execute (Mock Mode)

```bash
cargo run --example execute_proof -p katana_tee_client --release
```

### Generate Proof (Network)

```bash
SP1_PROVER=network cargo run --example generate_proof -p katana_tee_client --release
```

## Architecture

```
┌─────────────────────────────────────────────┐
│              katana_tee_client              │
│  - Katana RPC communication                 │
│  - TeeQuoteResponse types                   │
│  - CLI                                      │
└─────────────────────────────────────────────┘
                      │
                      │ uses
                      ▼
┌─────────────────────────────────────────────┐
│          amd_tee_registry_client            │
│  - AMD attestation parsing                  │
│  - SP1 proof generation                     │
│  - Certificate fetching from AMD KDS        │
└─────────────────────────────────────────────┘
                      │
                      │ uses
                      ▼
┌─────────────────────────────────────────────┐
│       amd-sev-snp-attestation-sdk           │
│  - Core SP1 program                         │
│  - On-chain proof types                     │
└─────────────────────────────────────────────┘
```

## Makefile Targets

The repository root includes a Makefile with convenient targets:

```bash
make fetch          # Fetch attestation
make execute        # Execute SP1 (mock)
make prove          # Generate proof (network)
make prove-mock     # Generate proof (mock)
make proof-info     # Show proof details
make help           # Show all targets
```

## License

Apache-2.0
