# AMD TEE Attestation Prover

Generate SP1 Groth16 proofs from AMD SEV-SNP attestation reports and convert them to Starknet calldata for on-chain verification using Garaga.

## Setup

This crate requires a local clone of the Garaga repository. Run the setup from the project root:

```bash
make setup-garaga
```

Or manually:
```bash
git clone --depth 1 https://github.com/keep-starknet-strange/garaga.git crates/garaga
```

## Usage

### Generate SP1 Proof

```rust
use amd_tee_registry_client::{AmdAttestationProver, ProverConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create prover (reads NETWORK_PRIVATE_KEY from environment)
    let prover = AmdAttestationProver::from_env();

    // Raw attestation report (1184 bytes)
    let report_bytes: Vec<u8> = get_attestation_report();

    // Generate Groth16 proof
    let proof = prover.prove(&report_bytes).await?;

    println!("Verifier ID: {}", proof.program_id.verifier_id);
    println!("Proof size: {} bytes", proof.onchain_proof.len());

    Ok(())
}
```

### Generate Starknet Calldata

```rust
use amd_tee_registry_client::{AmdAttestationProver, StarknetCalldata};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let prover = AmdAttestationProver::from_env();
    let proof = prover.prove(&report_bytes).await?;

    // Convert proof to Starknet calldata
    let calldata = StarknetCalldata::from_proof(&proof)?;

    // Get hex strings for contract calls
    println!("Calldata elements: {}", calldata.len());
    for hex in calldata.to_hex_strings() {
        println!("{}", hex);
    }

    // Or save to file for Starknet Foundry tests
    calldata.save_to_file(std::path::Path::new("calldata.txt"))?;

    Ok(())
}
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `SP1_PROVER` | Prover mode: `mock`, `cpu`, or `network` |
| `NETWORK_PRIVATE_KEY` | Private key for SP1 Prover Network |
| `SKIP_TIME_VALIDITY_CHECK` | Skip certificate time validation |

## API

### `AmdAttestationProver`

```rust
// Create from environment
let prover = AmdAttestationProver::from_env();

// Or with explicit config
let config = ProverConfig::new(
    Some("private_key".to_string()),
    None,  // Use default RPC URL
    false, // Don't skip time check
);
let prover = AmdAttestationProver::new(config);

// Generate proof
let proof = prover.prove(&report_bytes).await?;

// Verify proof structure
AmdAttestationProver::verify_proof_structure(&proof)?;
```

### `StarknetCalldata`

```rust
// Convert proof to calldata
let calldata = StarknetCalldata::from_proof(&proof)?;

// Access raw BigUint values
let values: &[BigUint] = calldata.values();

// Convert to hex strings (0x prefixed)
let hex_strings: Vec<String> = calldata.to_hex_strings();

// Convert to decimal strings
let decimal_strings: Vec<String> = calldata.to_decimal_strings();

// Get file content for Starknet Foundry read_txt()
let content: String = calldata.to_hex_file_content();

// Save to file
calldata.save_to_file(Path::new("calldata.txt"))?;
```

## Integration with Garaga

The generated calldata can be used with Garaga's SP1 verifier contract on Starknet. The calldata includes:

1. **Groth16 proof elements** (a, b, c points)
2. **SP1 program verification key**
3. **Public values** (the attestation report hash)
4. **Multi-Pairing Check (MPC) hints** for efficient verification
5. **Multi-Scalar Multiplication (MSM) hints**

## License

Apache-2.0
