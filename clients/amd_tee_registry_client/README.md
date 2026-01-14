# AMD TEE Attestation Prover

Generate SP1 Groth16 proofs from AMD SEV-SNP attestation reports for on-chain verification.

## Usage

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

## License

Apache-2.0
