//! SP1 Proof Generation for Katana TEE Attestations
//!
//! This module provides functionality to generate SP1 Groth16 proofs
//! from AMD SEV-SNP attestation quotes.

use crate::{Error, TeeQuoteResponse};
use alloy_primitives::Bytes;
use amd_sev_snp_attestation_prover::{AmdSevSnpProver, OnchainProof, ProverConfig};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

/// Configuration for SP1 proof generation.
#[derive(Debug, Clone, Default)]
pub struct Sp1ProverConfig {
    /// SP1 private key for network proving (optional for mock mode)
    pub private_key: Option<String>,
    /// SP1 RPC URL (optional, uses default if not specified)
    pub rpc_url: Option<String>,
    /// Skip time validity check (useful for testing with old attestations)
    pub skip_time_validity_check: bool,
}

impl Sp1ProverConfig {
    /// Create config from environment variables.
    ///
    /// Reads:
    /// - `NETWORK_PRIVATE_KEY` - Private key for SP1 network proving (preferred)
    /// - `SP1_PRIVATE_KEY` - Private key for network proving (fallback)
    /// - `SP1_RPC_URL` - RPC URL for SP1 network
    /// - `SKIP_TIME_VALIDITY_CHECK` - Skip time validity (true/false)
    pub fn from_env() -> Self {
        Self {
            // NETWORK_PRIVATE_KEY is the standard env var used by SP1 SDK
            private_key: std::env::var("NETWORK_PRIVATE_KEY")
                .ok()
                .or_else(|| std::env::var("SP1_PRIVATE_KEY").ok()),
            rpc_url: std::env::var("SP1_RPC_URL").ok(),
            skip_time_validity_check: std::env::var("SKIP_TIME_VALIDITY_CHECK")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(false),
        }
    }
}

/// Generate an SP1 Groth16 proof from a Katana TEE attestation quote.
///
/// This function:
/// 1. Decodes the hex quote from the response
/// 2. Parses it as an AMD SEV-SNP attestation report
/// 3. Fetches the certificate chain from AMD KDS
/// 4. Generates a Groth16 proof using the SP1 prover
///
/// # Arguments
/// * `response` - The TEE quote response from Katana RPC
///
/// # Environment Variables
/// * `SP1_PROVER` - Set to "mock" for local testing, "network" for real proving
/// * `SP1_PRIVATE_KEY` - Required for network proving
///
/// # Example
/// ```no_run
/// use katana_tee_client::{TeeQuoteResponse, generate_sp1_proof};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let response = TeeQuoteResponse::from_json_file("example_response.json".as_ref())?;
/// let proof = generate_sp1_proof(response).await?;
/// println!("Generated proof: {:?}", proof.program_id);
/// # Ok(())
/// # }
/// ```
pub async fn generate_sp1_proof(response: TeeQuoteResponse) -> Result<OnchainProof, Error> {
    generate_sp1_proof_with_config(response, Sp1ProverConfig::from_env()).await
}

/// Generate an SP1 Groth16 proof with custom configuration.
///
/// # Arguments
/// * `response` - The TEE quote response from Katana RPC
/// * `config` - Custom prover configuration
pub async fn generate_sp1_proof_with_config(
    response: TeeQuoteResponse,
    config: Sp1ProverConfig,
) -> Result<OnchainProof, Error> {
    info!(
        "Starting SP1 proof generation for block {}",
        response.block_number
    );

    // Set environment variables for SP1 prover
    if let Some(ref key) = config.private_key {
        std::env::set_var("SP1_PRIVATE_KEY", key);
    }
    if let Some(ref url) = config.rpc_url {
        std::env::set_var("SP1_RPC_URL", url);
    }
    if config.skip_time_validity_check {
        std::env::set_var("SKIP_TIME_VALIDITY_CHECK", "true");
    }

    // Decode quote bytes
    let quote_bytes = response.quote_bytes()?;
    debug!("Decoded quote: {} bytes", quote_bytes.len());

    // Validate quote size (AMD SEV-SNP attestation report is 1184 bytes)
    const EXPECTED_REPORT_SIZE: usize = 1184;
    if quote_bytes.len() != EXPECTED_REPORT_SIZE {
        return Err(Error::AttestationReport(format!(
            "Invalid quote size: expected {} bytes, got {}",
            EXPECTED_REPORT_SIZE,
            quote_bytes.len()
        )));
    }

    // Get current timestamp
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Prover(format!("Failed to get timestamp: {}", e)))?
        .as_secs();

    debug!("Using timestamp: {}", timestamp);

    // Run the blocking prover in a separate thread to avoid tokio runtime conflicts
    // The AMD SDK prover uses its own internal tokio runtime via block_on
    let proof = tokio::task::spawn_blocking(move || {
        // Create prover with SP1 configuration
        let prover_config = ProverConfig::sp1();
        let prover = AmdSevSnpProver::new(prover_config, None);

        info!("SP1 prover initialized, generating proof...");

        // Generate the proof
        // Note: vek_certs=None means the prover will fetch them from AMD KDS
        prover.prove_attestation_report(timestamp, Bytes::from(quote_bytes), None)
    })
    .await
    .map_err(|e| Error::Prover(format!("Task join error: {}", e)))?
    .map_err(|e| Error::Prover(format!("Proof generation failed: {}", e)))?;

    info!(
        "Proof generated successfully. Program ID: {:?}",
        proof.program_id.verifier_id
    );

    Ok(proof)
}

/// Verify a generated proof locally (without on-chain verification).
///
/// This is useful for testing to ensure the proof structure is valid.
pub fn verify_proof_structure(proof: &OnchainProof) -> Result<(), Error> {
    // Check that we have an actual proof
    if proof.onchain_proof.is_empty() {
        return Err(Error::Prover("Proof bytes are empty".to_string()));
    }

    // Check program ID is set
    if proof.program_id.verifier_id.is_zero() {
        return Err(Error::Prover("Verifier ID is zero".to_string()));
    }

    info!(
        "Proof structure valid. Type: {:?}, Size: {} bytes",
        proof.zktype,
        proof.onchain_proof.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require the SP1 prover to be properly configured
    // Run with: SP1_PROVER=mock cargo test

    #[test]
    fn test_config_from_env() {
        std::env::set_var("SP1_PRIVATE_KEY", "test_key");
        std::env::set_var("SKIP_TIME_VALIDITY_CHECK", "true");

        let config = Sp1ProverConfig::from_env();
        assert_eq!(config.private_key, Some("test_key".to_string()));
        assert!(config.skip_time_validity_check);

        // Clean up
        std::env::remove_var("SP1_PRIVATE_KEY");
        std::env::remove_var("SKIP_TIME_VALIDITY_CHECK");
    }
}
