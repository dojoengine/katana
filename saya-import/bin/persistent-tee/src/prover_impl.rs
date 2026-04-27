//! TEE attestation handling and SP1 proof generation.
//!
//! This module wraps the AMD TEE attestation prover from katana-tee
//! to generate verifiable proofs of TEE execution.
//!
//! We run KDS cert fetch in a plain OS thread (no tokio) so that reqwest::blocking
//! inside KDS does not create a nested runtime; then we create a tokio runtime
//! only for registry lookup and proof generation.

use anyhow::Result;
use katana_tee_client::{OnchainProof, ProverConfig, StarknetRegistryClient};
use starknet_types_core::felt::Felt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

/// Structured error type for TEE attestation operations.
#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error("invalid attestation report: {0}")]
    InvalidReport(String),
    #[error("proof generation failed: {0}")]
    ProofGenerationFailed(String),
    #[error("proof generation timed out after {0:?}")]
    Timeout(Duration),
    #[error("proof generation thread panicked")]
    ThreadPanicked,
    #[error("{0}")]
    Other(String),
}

/// Maximum time for the entire proof generation pipeline (KDS + registry + SP1).
const PROOF_GENERATION_TIMEOUT: Duration = Duration::from_secs(600);

use alloy_primitives::Bytes;
use amd_sev_snp_attestation_prover::{
    AmdSevSnpProver, ProverConfig as SdkProverConfig, RawProofType, SP1ProverConfig, KDS,
};
use amd_sev_snp_attestation_verifier::{stub::ProcessorType, AttestationReport};
use amd_tee_registry_client::{
    prepare_verifier_input_with_storage, report::AttestationReportBytes,
};
use x509_verifier_rust_crypto::CertChain;

/// TEE attestation with proof generation capabilities.
pub struct TeeAttestation {
    /// 1184 bytes for AMD SEV-SNP.
    quote_bytes: Vec<u8>,
    block_number: Felt,
}

impl TeeAttestation {
    pub fn from_response(
        response: &katana_tee_client::TeeQuoteResponse,
    ) -> Result<Self, AttestationError> {
        let quote_bytes = response
            .quote_bytes()
            .map_err(|e| AttestationError::InvalidReport(e.to_string()))?;
        Ok(Self {
            quote_bytes,
            block_number: response.block_number,
        })
    }

    /// Generate SP1 Groth16 proof for this attestation.
    pub async fn generate_proof(
        &self,
        provider_url: &str,
        registry_address: Felt,
        prover_config: ProverConfig,
    ) -> Result<OnchainProof, AttestationError> {
        self.generate_proof_with_storage(provider_url, registry_address, prover_config)
            .await
    }

    /// Generate SP1 Groth16 proof for this attestation, optionally including storage/event proofs.
    ///
    /// Architecture: we spawn a dedicated OS thread because `reqwest::blocking` inside KDS
    /// panics if called from within a tokio runtime.
    pub async fn generate_proof_with_storage(
        &self,
        provider_url: &str,
        registry_address: Felt,
        prover_config: ProverConfig,
    ) -> Result<OnchainProof, AttestationError> {
        let mode = "network";
        info!(
            "{} {} {}",
            self.block_number, mode, "Generating SP1 proof for TEE attestation"
        );

        let quote_bytes = self.quote_bytes.clone();
        let provider_url = provider_url.to_string();

        let handle = std::thread::spawn(move || -> Result<OnchainProof> {
            // Phase 1: KDS cert fetch — must run outside tokio.
            let report = AttestationReportBytes::new(&quote_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid attestation report: {e}"))?;
            let report_struct = AttestationReport::from_bytes(report.as_bytes())
                .map_err(|e| anyhow::anyhow!("Attestation report parse failed: {e}"))?;
            let processor_model_u8 = match report_struct
                .get_cpu_codename()
                .map_err(|e| anyhow::anyhow!("Processor model error: {e}"))?
            {
                ProcessorType::Milan => 0u8,
                ProcessorType::Genoa => 1,
                ProcessorType::Bergamo => 2,
                ProcessorType::Siena => 3,
                other => anyhow::bail!("Unsupported processor model: {other:?}"),
            };
            let kds_chain = KDS::new()
                .fetch_report_cert_chain(&report_struct)
                .map_err(|e| anyhow::anyhow!("KDS cert chain fetch failed: {e}"))?;
            let cert_chain = CertChain::parse_rev(&kds_chain)
                .map_err(|e| anyhow::anyhow!("Cert chain parse failed: {e}"))?;
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .map_err(|e| anyhow::anyhow!("System time error: {e}"))?;

            // Phase 2: Registry lookup + proof generation (needs async).
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to create tokio runtime: {e}"))?;

            rt.block_on(async {
                let registry_client = StarknetRegistryClient::new(&provider_url, registry_address);
                let trusted_prefix_len = registry_client
                    .fetch_trusted_prefix_len(processor_model_u8, cert_chain.digest())
                    .await
                    .map_err(|e| anyhow::anyhow!("Registry fetch failed: {e}"))?;

                if !prover_config.skip_time_validity_check {
                    cert_chain
                        .check_valid(timestamp)
                        .map_err(|e| anyhow::anyhow!("Cert chain time validation failed: {e}"))?;
                }

                let report_bytes = quote_bytes.clone();
                let vek_der_chain = cert_chain.to_ders();
                let sp1_config = SP1ProverConfig {
                    private_key: prover_config.private_key.clone(),
                    rpc_url: prover_config.rpc_url.clone(),
                    prover_mode: Some("network".to_string()),
                };
                let mut sdk_config = SdkProverConfig::sp1_with(sp1_config);
                sdk_config.skip_time_validity_check = prover_config.skip_time_validity_check;

                tokio::task::spawn_blocking(move || {
                    let prover = AmdSevSnpProver::new(sdk_config, None);
                    let input: amd_sev_snp_attestation_verifier::stub::VerifierInput =
                        prepare_verifier_input_with_storage(
                            timestamp,
                            Bytes::from(report_bytes),
                            vek_der_chain,
                            trusted_prefix_len,
                            None,
                            None,
                        );
                    debug!("{:?} {}", input, "SP1 Groth16 prover input");
                    let raw_proof = prover
                        .verifier
                        .gen_proof(&input, RawProofType::Groth16, None)
                        .map_err(|e| anyhow::anyhow!("Proof generation failed: {e}"))?;
                    prover
                        .create_onchain_proof(raw_proof)
                        .map_err(|e| anyhow::anyhow!("Onchain proof creation failed: {e}"))
                })
                .await
                .map_err(|e| anyhow::anyhow!("SP1 proof task panicked: {e}"))?
            })
        });

        let proof = tokio::time::timeout(
            PROOF_GENERATION_TIMEOUT,
            tokio::task::spawn_blocking(move || {
                handle
                    .join()
                    .map_err(|_| AttestationError::ThreadPanicked)?
                    .map_err(|e| AttestationError::ProofGenerationFailed(e.to_string()))
            }),
        )
        .await
        .map_err(|_| AttestationError::Timeout(PROOF_GENERATION_TIMEOUT))?
        .map_err(|e| AttestationError::Other(format!("spawn_blocking join failed: {e}")))??;

        info!("SP1 proof generated successfully");
        Ok(proof)
    }
}
