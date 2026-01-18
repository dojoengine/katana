//! SP1 Proof Generation for Katana TEE Attestations
//!
//! This module provides Katana-specific proof generation functionality,
//! wrapping the generic AMD attestation prover from `amd_tee_registry_client`.
//!
//! # Example
//!
//! ```no_run
//! use katana_tee_client::{TeeQuoteResponse, generate_sp1_proof};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let response = TeeQuoteResponse::from_json_file("example_response.json".as_ref())?;
//! let proof = generate_sp1_proof(response).await?;
//! println!("Generated proof: {:?}", proof.program_id);
//! # Ok(())
//! # }
//! ```

use crate::{Error, TeeQuoteResponse};
use tracing::info;

// Re-export from amd_tee_registry_client for convenience
pub use amd_tee_registry_client::{AmdAttestationProver, OnchainProof, ProverConfig};
use amd_tee_registry_client::StarknetRegistryClient;

/// Generate an SP1 Groth16 proof from a Katana TEE attestation quote.
///
/// This function:
/// 1. Decodes the hex quote from the response
/// 2. Delegates to `amd_tee_registry_client::AmdAttestationProver` for proof generation
///
/// # Arguments
/// * `response` - The TEE quote response from Katana RPC
///
/// # Environment Variables
/// * `SP1_PROVER` - Set to "mock" for local testing, "network" for real proving
/// * `NETWORK_PRIVATE_KEY` - Required for network proving (SP1 Prover Network)
pub async fn generate_sp1_proof(response: TeeQuoteResponse) -> Result<OnchainProof, Error> {
    generate_sp1_proof_with_config(response, ProverConfig::from_env()).await
}

/// Generate an SP1 Groth16 proof with custom configuration.
///
/// # Arguments
/// * `response` - The TEE quote response from Katana RPC
/// * `config` - Custom prover configuration
pub async fn generate_sp1_proof_with_config(
    response: TeeQuoteResponse,
    config: ProverConfig,
) -> Result<OnchainProof, Error> {
    info!(
        "Generating SP1 proof for Katana block {}",
        response.block_number
    );

    // Decode quote bytes from hex
    let quote_bytes = response.quote_bytes()?;

    // Use the generic AMD attestation prover
    let prover = AmdAttestationProver::new(config);
    let proof = prover
        .prove(&quote_bytes)
        .await
        .map_err(|e| Error::Prover(e.to_string()))?;

    info!(
        "Proof generated for block {}. Verifier ID: {}",
        response.block_number, proof.program_id.verifier_id
    );

    Ok(proof)
}

/// Generate an SP1 Groth16 proof using on-chain cache information.
///
/// This queries the AMD TEE registry on Starknet for the trusted certificate prefix length
/// and injects it into the SP1 verifier input before proving.
pub async fn generate_sp1_proof_with_cache(
    response: TeeQuoteResponse,
    config: ProverConfig,
    registry_client: &StarknetRegistryClient,
) -> Result<OnchainProof, Error> {
    info!(
        "Generating SP1 proof (with cache) for Katana block {}",
        response.block_number
    );

    let quote_bytes = response.quote_bytes()?;
    let prover = AmdAttestationProver::new(config);

    let proof = prover
        .prove_with_cache(&quote_bytes, registry_client)
        .await
        .map_err(|e| Error::Prover(e.to_string()))?;

    Ok(proof)
}

/// Verify a generated proof has valid structure (without on-chain verification).
///
/// This is useful for testing to ensure the proof structure is valid.
pub fn verify_proof_structure(proof: &OnchainProof) -> Result<(), Error> {
    AmdAttestationProver::verify_proof_structure(proof).map_err(|e| Error::Prover(e.to_string()))
}
