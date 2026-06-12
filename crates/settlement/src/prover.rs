//! Turns a [`BlockAttestation`] into the `TEEInput.sp1_proof` payload.
//!
//! - [`TeeProver::Sp1`] runs the real pipeline: AMD KDS certificate chain fetch, on-chain
//!   trusted-prefix lookup against the TEE registry, SP1 Groth16 proving on the prover network, and
//!   Garaga calldata conversion.
//! - [`TeeProver::Mock`] synthesizes a stub `VerifierJournal` carrying the same v1 commitment a
//!   real proof would attest to — paired with the `piltover_mock_amd_tee_registry` contract for
//!   test environments without SEV-SNP hardware.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alloy_primitives::Bytes;
use amd_sev_snp_attestation_prover::{
    AmdSevSnpProver, ProverConfig as SdkProverConfig, RawProofType, SP1ProverConfig, KDS,
};
use amd_sev_snp_attestation_verifier::stub::ProcessorType;
use amd_sev_snp_attestation_verifier::AttestationReport;
use anyhow::anyhow;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::tee::BlockAttestation;
use katana_tee::amd::report::AttestationReportBytes;
use katana_tee::amd::{prepare_verifier_input_with_storage, OnchainProof, StarknetRegistryClient};
use katana_tee::attestation::compute_appchain_commitment;
use tracing::{debug, info};
use url::Url;
use x509_verifier_rust_crypto::CertChain;

use crate::{mock, LOG_TARGET};

/// Maximum time for the entire proof generation pipeline (KDS + registry + SP1).
const PROOF_GENERATION_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("invalid attestation report: {0}")]
    InvalidReport(String),

    #[error("proof generation failed: {0}")]
    ProofGenerationFailed(String),

    #[error("proof generation timed out after {0:?}")]
    Timeout(Duration),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Proof backend for the settlement service.
#[derive(Debug)]
pub enum TeeProver {
    /// Synthesizes mock `VerifierJournal` payloads. Only valid against a Piltover core whose
    /// fact registry is the permissive `piltover_mock_amd_tee_registry`.
    Mock,

    /// Real SP1 Groth16 proving of AMD SEV-SNP attestations via the prover network.
    Sp1 {
        /// Settlement chain JSON-RPC endpoint (for the TEE registry lookup).
        settlement_rpc: Url,
        /// AMD TEE registry contract on the settlement chain.
        tee_registry: ContractAddress,
        /// SP1 prover-network private key.
        prover_key: String,
    },
}

impl TeeProver {
    /// Produces the felts for `TEEInput.sp1_proof` from the given attestation.
    pub async fn prove(&self, attestation: &BlockAttestation) -> Result<Vec<Felt>, ProverError> {
        match self {
            Self::Mock => {
                // The mock journal carries the exact v1 commitment Piltover recomputes from the
                // `TEEInput` fields — same formula as the attestation's `report_data`.
                let commitment = compute_appchain_commitment(
                    attestation.prev_state_root,
                    attestation.state_root,
                    attestation.prev_block_hash,
                    attestation.block_hash,
                    attestation.prev_block_number,
                    attestation.block_number,
                    attestation.messages_commitment,
                    attestation.katana_tee_config_hash,
                );

                debug!(target: LOG_TARGET, %commitment, "Synthesized mock proof journal.");

                Ok(mock::serialize_mock_journal(commitment, attestation.katana_tee_config_hash))
            }

            Self::Sp1 { settlement_rpc, tee_registry, prover_key } => {
                let proof = tokio::time::timeout(
                    PROOF_GENERATION_TIMEOUT,
                    self.generate_sp1_proof(attestation, settlement_rpc, *tee_registry, prover_key),
                )
                .await
                .map_err(|_| ProverError::Timeout(PROOF_GENERATION_TIMEOUT))??;

                onchain_proof_to_calldata(&proof).map_err(ProverError::Other)
            }
        }
    }

    /// The real proving pipeline.
    ///
    /// The KDS certificate fetch (`reqwest::blocking` inside the SDK) and the SP1 proving call
    /// are blocking, so both run on the blocking thread pool; the registry lookup in between is
    /// natively async.
    async fn generate_sp1_proof(
        &self,
        attestation: &BlockAttestation,
        settlement_rpc: &Url,
        tee_registry: ContractAddress,
        prover_key: &str,
    ) -> Result<OnchainProof, ProverError> {
        let quote = attestation.quote.strip_prefix("0x").unwrap_or(&attestation.quote);
        let quote = hex::decode(quote).map_err(|e| ProverError::InvalidReport(e.to_string()))?;

        info!(
            target: LOG_TARGET,
            block_number = %attestation.block_number,
            "Generating SP1 proof for TEE attestation."
        );

        // Phase 1: report parsing + KDS cert chain fetch — blocking (the KDS client uses
        // `reqwest::blocking`, which panics inside an async context).
        let quote_bytes = quote.clone();
        let (processor_model, vek_der_chain, cert_digests) =
            tokio::task::spawn_blocking(move || -> Result<_, ProverError> {
                let report = AttestationReportBytes::new(&quote_bytes)
                    .map_err(|e| ProverError::InvalidReport(e.to_string()))?;
                let report = AttestationReport::from_bytes(report.as_bytes())
                    .map_err(|e| ProverError::InvalidReport(e.to_string()))?;

                let processor_model = match report
                    .get_cpu_codename()
                    .map_err(|e| ProverError::InvalidReport(e.to_string()))?
                {
                    ProcessorType::Milan => 0u8,
                    ProcessorType::Genoa => 1,
                    ProcessorType::Bergamo => 2,
                    ProcessorType::Siena => 3,
                    other => {
                        return Err(ProverError::InvalidReport(format!(
                            "unsupported processor model: {other:?}"
                        )))
                    }
                };

                let kds_chain = KDS::new()
                    .fetch_report_cert_chain(&report)
                    .map_err(|e| anyhow!("KDS cert chain fetch failed: {e}"))?;
                let cert_chain = CertChain::parse_rev(&kds_chain)
                    .map_err(|e| anyhow!("cert chain parse failed: {e}"))?;

                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .map_err(|e| anyhow!("system time error: {e}"))?;
                cert_chain
                    .check_valid(timestamp)
                    .map_err(|e| anyhow!("cert chain time validation failed: {e}"))?;

                Ok((processor_model, cert_chain.to_ders(), cert_chain.digest().to_vec()))
            })
            .await
            .map_err(|e| ProverError::Other(anyhow!("KDS task panicked: {e}")))??;

        // Phase 2: trusted-prefix lookup against the on-chain TEE registry — natively async.
        let registry_client = StarknetRegistryClient::new(settlement_rpc.as_str(), tee_registry);
        let trusted_prefix_len = registry_client
            .fetch_trusted_prefix_len(processor_model, &cert_digests)
            .await
            .map_err(|e| anyhow!("TEE registry fetch failed: {e}"))?;

        // Phase 3: SP1 Groth16 proving — blocking (CPU + synchronous network calls in the SDK).
        let prover_key = prover_key.to_string();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .map_err(|e| ProverError::Other(anyhow!("system time error: {e}")))?;

        tokio::task::spawn_blocking(move || -> Result<OnchainProof, ProverError> {
            let sp1_config = SP1ProverConfig {
                private_key: Some(prover_key),
                rpc_url: None,
                prover_mode: Some("network".to_string()),
            };
            let sdk_config = SdkProverConfig::sp1_with(sp1_config);

            let prover = AmdSevSnpProver::new(sdk_config, None);
            let input = prepare_verifier_input_with_storage(
                timestamp,
                Bytes::from(quote),
                vek_der_chain,
                trusted_prefix_len,
                None,
                None,
            );
            debug!(target: LOG_TARGET, ?input, "SP1 Groth16 prover input.");

            let raw_proof = prover
                .verifier
                .gen_proof(&input, RawProofType::Groth16, None)
                .map_err(|e| ProverError::ProofGenerationFailed(e.to_string()))?;
            prover
                .create_onchain_proof(raw_proof)
                .map_err(|e| ProverError::ProofGenerationFailed(e.to_string()))
        })
        .await
        .map_err(|e| ProverError::Other(anyhow!("SP1 proof task panicked: {e}")))?
    }
}

/// Converts an [`OnchainProof`] into the Garaga Groth16 calldata felts that Piltover's SP1
/// verifier expects in `TEEInput.sp1_proof`.
fn onchain_proof_to_calldata(proof: &OnchainProof) -> Result<Vec<Felt>, anyhow::Error> {
    use garaga_rs::calldata::full_proof_with_hints::groth16::{
        get_groth16_calldata, get_sp1_vk, Groth16Proof,
    };
    use garaga_rs::definitions::CurveID;

    if proof.onchain_proof.is_empty() {
        return Err(anyhow!("cannot generate calldata from empty proof"));
    }

    // The verifier_id is the SP1 program vkey as bytes; the public values are in the raw_proof
    // journal; the onchain_proof contains the Groth16 proof with its 4-byte selector.
    let vkey_bytes = proof.program_id.verifier_id.as_slice().to_vec();
    let public_values = proof.raw_proof.journal.to_vec();
    let proof_bytes = proof.onchain_proof.to_vec();

    let groth16_proof = Groth16Proof::from_sp1(vkey_bytes, public_values, proof_bytes);
    let sp1_vk = get_sp1_vk();

    let calldata = get_groth16_calldata(&groth16_proof, &sp1_vk, CurveID::BN254)
        .map_err(|e| anyhow!("failed to generate calldata: {e}"))?;

    Ok(calldata.into_iter().map(Felt::from).collect())
}
