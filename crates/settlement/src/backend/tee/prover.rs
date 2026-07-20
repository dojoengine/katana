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
use katana_primitives::{ContractAddress, Felt, B256};
use katana_rpc_types::tee::BlockAttestation;
use katana_tee::amd::report::AttestationReportBytes;
use katana_tee::amd::{prepare_verifier_input_with_storage, OnchainProof, StarknetRegistryClient};
use katana_tee::attestation::compute_appchain_commitment;
use tracing::{debug, info};
use url::Url;
use x509_verifier_rust_crypto::CertChain;

use super::mock;

/// Maximum time for the entire proof generation pipeline (KDS + registry + SP1).
const PROOF_GENERATION_TIMEOUT: Duration = Duration::from_secs(600);

/// Maximum time to wait when recovering an already-fulfilled proof from the prover network by its
/// request id. Deliberately short: a fulfilled proof returns immediately, so anything slower
/// means the id is unknown/expired and the caller should fall back to fresh proving.
const PROOF_RECOVERY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum TeeProverError {
    #[error("invalid attestation report: {0}")]
    InvalidReport(String),

    #[error("KDS certificate / TEE registry error: {0}")]
    Kds(String),

    #[error("system time error: {0}")]
    SystemTime(String),

    #[error("proof generation failed: {0}")]
    ProofGenerationFailed(String),

    #[error("proof generation timed out after {0:?}")]
    Timeout(Duration),

    #[error("calldata generation failed: {0}")]
    Calldata(String),
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
    /// Produces the felts for `TEEInput.sp1_proof` from the given attestation, along with the
    /// Succinct prover-network request ID of the proof (`None` for mock / off-network proving).
    pub async fn prove(
        &self,
        attestation: &BlockAttestation,
    ) -> Result<(Vec<Felt>, Option<B256>), TeeProverError> {
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

                debug!(%commitment, "Synthesized mock proof journal.");

                Ok((
                    mock::serialize_mock_journal(commitment, attestation.katana_tee_config_hash),
                    None,
                ))
            }

            Self::Sp1 { settlement_rpc, tee_registry, prover_key } => {
                let proof = tokio::time::timeout(
                    PROOF_GENERATION_TIMEOUT,
                    self.generate_sp1_proof(attestation, settlement_rpc, *tee_registry, prover_key),
                )
                .await
                .map_err(|_| TeeProverError::Timeout(PROOF_GENERATION_TIMEOUT))??;

                record_proof_cost(&proof, attestation);
                let request_id = proof.raw_proof.request_id;
                let calldata = onchain_proof_to_calldata(&proof)?;
                Ok((calldata, request_id))
            }
        }
    }

    /// Recovers the `TEEInput.sp1_proof` felts of an already-generated proof from the prover
    /// network by its request id, without submitting (and paying for) a new proving request.
    ///
    /// Returns `Ok(None)` for the mock prover — it has no network to recover from, and mock
    /// proving is free anyway. `Err` means the network no longer serves the request (expired /
    /// unknown id) or the fetch failed; the caller falls back to fresh proving.
    pub async fn recover(
        &self,
        proof: &katana_primitives::settlement::ProofId,
    ) -> Result<Option<Vec<Felt>>, TeeProverError> {
        match self {
            Self::Mock => Ok(None),

            Self::Sp1 { prover_key, .. } => {
                let request_id = B256::try_from(proof.0.as_ref()).map_err(|_| {
                    TeeProverError::ProofGenerationFailed(format!(
                        "proof id is not a 32-byte SP1 request id ({} bytes)",
                        proof.0.len()
                    ))
                })?;

                info!(
                    request_id = %format!("{request_id:#x}"),
                    "Recovering SP1 proof from the prover network."
                );

                // Blocking for the same reason as proving: the SDK drives the network client
                // through its own `block_on`.
                let prover_key = prover_key.to_string();
                let proof = tokio::task::spawn_blocking(move || -> Result<_, TeeProverError> {
                    let sp1_config = SP1ProverConfig {
                        private_key: Some(prover_key),
                        rpc_url: None,
                        prover_mode: Some("network".to_string()),
                    };
                    let prover = AmdSevSnpProver::new(SdkProverConfig::sp1_with(sp1_config), None);

                    let raw_proof = prover
                        .verifier
                        .recover_proof(request_id, Some(PROOF_RECOVERY_TIMEOUT))
                        .map_err(|e| TeeProverError::ProofGenerationFailed(e.to_string()))?;
                    prover
                        .create_onchain_proof(raw_proof)
                        .map_err(|e| TeeProverError::ProofGenerationFailed(e.to_string()))
                })
                .await
                .map_err(|e| {
                    TeeProverError::ProofGenerationFailed(format!(
                        "SP1 proof recovery task panicked: {e}"
                    ))
                })??;

                Ok(Some(onchain_proof_to_calldata(&proof)?))
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
    ) -> Result<OnchainProof, TeeProverError> {
        let quote = attestation.quote.strip_prefix("0x").unwrap_or(&attestation.quote);
        let quote = hex::decode(quote).map_err(|e| TeeProverError::InvalidReport(e.to_string()))?;

        info!(
            block_number = %attestation.block_number,
            "Generating SP1 proof for TEE attestation."
        );

        // Phase 1: report parsing + KDS cert chain fetch — blocking (the KDS client uses
        // `reqwest::blocking`, which panics inside an async context).
        let quote_bytes = quote.clone();
        let (processor_model, vek_der_chain, cert_digests) =
            tokio::task::spawn_blocking(move || -> Result<_, TeeProverError> {
                let report = AttestationReportBytes::new(&quote_bytes)
                    .map_err(|e| TeeProverError::InvalidReport(e.to_string()))?;
                let report = AttestationReport::from_bytes(report.as_bytes())
                    .map_err(|e| TeeProverError::InvalidReport(e.to_string()))?;

                let processor_model = match report
                    .get_cpu_codename()
                    .map_err(|e| TeeProverError::InvalidReport(e.to_string()))?
                {
                    ProcessorType::Milan => 0u8,
                    ProcessorType::Genoa => 1,
                    ProcessorType::Bergamo => 2,
                    ProcessorType::Siena => 3,
                    other => {
                        return Err(TeeProverError::InvalidReport(format!(
                            "unsupported processor model: {other:?}"
                        )))
                    }
                };

                let kds_chain = KDS::new()
                    .fetch_report_cert_chain(&report)
                    .map_err(|e| TeeProverError::Kds(format!("cert chain fetch failed: {e}")))?;
                let cert_chain = CertChain::parse_rev(&kds_chain)
                    .map_err(|e| TeeProverError::Kds(format!("cert chain parse failed: {e}")))?;

                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .map_err(|e| TeeProverError::SystemTime(e.to_string()))?;
                cert_chain.check_valid(timestamp).map_err(|e| {
                    TeeProverError::Kds(format!("cert chain time validation failed: {e}"))
                })?;

                Ok((processor_model, cert_chain.to_ders(), cert_chain.digest().to_vec()))
            })
            .await
            .map_err(|e| TeeProverError::Kds(format!("KDS task panicked: {e}")))??;

        // Phase 2: trusted-prefix lookup against the on-chain TEE registry — natively async.
        let registry_client = StarknetRegistryClient::new(settlement_rpc.as_str(), tee_registry);
        let trusted_prefix_len = registry_client
            .fetch_trusted_prefix_len(processor_model, &cert_digests)
            .await
            .map_err(|e| TeeProverError::Kds(format!("TEE registry fetch failed: {e}")))?;

        // Phase 3: SP1 Groth16 proving — blocking (CPU + synchronous network calls in the SDK).
        let prover_key = prover_key.to_string();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .map_err(|e| TeeProverError::SystemTime(e.to_string()))?;

        tokio::task::spawn_blocking(move || -> Result<OnchainProof, TeeProverError> {
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
            debug!(?input, "SP1 Groth16 prover input.");

            let raw_proof = prover
                .verifier
                .gen_proof(&input, RawProofType::Groth16, None)
                .map_err(|e| TeeProverError::ProofGenerationFailed(e.to_string()))?;
            prover
                .create_onchain_proof(raw_proof)
                .map_err(|e| TeeProverError::ProofGenerationFailed(e.to_string()))
        })
        .await
        .map_err(|e| {
            TeeProverError::ProofGenerationFailed(format!("SP1 proof task panicked: {e}"))
        })?
    }
}

/// Records the proof-generation cost, when the SP1 prover network reported it (the mock prover and
/// off-network proving report no cost, so this is a no-op there). Recorded here — the only place
/// the [`OnchainProof`] and its cost are in scope — rather than in the backend-agnostic settlement
/// service.
///
/// The cost is emitted two ways: as aggregate `katana_settlement_sp1_proof_*` histograms (for
/// dashboards and alerting), and as a per-proof `info` log carrying the settled block range. The
/// metrics carry no block label on purpose — a block number is unbounded cardinality — so the log
/// is how you attribute a specific cost to the batch `(prev_block, block]` that incurred it.
fn record_proof_cost(proof: &OnchainProof, attestation: &BlockAttestation) {
    use std::sync::Once;

    use metrics::{describe_histogram, histogram};

    static DESCRIBE: Once = Once::new();
    DESCRIBE.call_once(|| {
        describe_histogram!(
            "settlement.sp1_proof_cycles",
            "RISC-V cycles executed to generate an SP1 settlement proof."
        );
        describe_histogram!(
            "settlement.sp1_proof_gas_used",
            "Prover-network gas (PGU) billed for an SP1 settlement proof."
        );
        describe_histogram!(
            "settlement.sp1_proof_gas_price",
            "Prover-network gas price (PROVE base units per PGU) at settlement time."
        );
    });

    let Some(cost) = &proof.raw_proof.cost else { return };

    if let Some(cycles) = cost.cycles {
        histogram!("settlement.sp1_proof_cycles").record(cycles as f64);
    }
    if let Some(gas_used) = cost.gas_used {
        histogram!("settlement.sp1_proof_gas_used").record(gas_used as f64);
    }
    if let Some(gas_price) = cost.gas_price {
        histogram!("settlement.sp1_proof_gas_price").record(gas_price as f64);
    }

    // Per-range attribution: `prev_block` is `None` for the genesis batch (its `Felt::MAX`
    // sentinel does not fit a `u64`).
    let block = u64::try_from(attestation.block_number).unwrap_or_default();
    let prev_block = u64::try_from(attestation.prev_block_number).ok();
    info!(
        ?prev_block,
        block,
        cycles = ?cost.cycles,
        gas_used = ?cost.gas_used,
        gas_price = ?cost.gas_price,
        deduction_amount = ?cost.deduction_amount,
        "SP1 proof generation cost."
    );
}

/// Converts an [`OnchainProof`] into the Garaga Groth16 calldata felts that Piltover's SP1
/// verifier expects in `TEEInput.sp1_proof`.
fn onchain_proof_to_calldata(proof: &OnchainProof) -> Result<Vec<Felt>, TeeProverError> {
    use garaga_rs::calldata::full_proof_with_hints::groth16::{
        get_groth16_calldata, get_sp1_vk, Groth16Proof,
    };
    use garaga_rs::definitions::CurveID;

    if proof.onchain_proof.is_empty() {
        return Err(TeeProverError::Calldata("cannot generate calldata from empty proof".into()));
    }

    // The verifier_id is the SP1 program vkey as bytes; the public values are in the raw_proof
    // journal; the onchain_proof contains the Groth16 proof with its 4-byte selector.
    let vkey_bytes = proof.program_id.verifier_id.as_slice().to_vec();
    let public_values = proof.raw_proof.journal.to_vec();
    let proof_bytes = proof.onchain_proof.to_vec();

    let groth16_proof = Groth16Proof::from_sp1(vkey_bytes, public_values, proof_bytes);
    let sp1_vk = get_sp1_vk();

    let calldata = get_groth16_calldata(&groth16_proof, &sp1_vk, CurveID::BN254)
        .map_err(|e| TeeProverError::Calldata(format!("failed to generate calldata: {e}")))?;

    Ok(calldata.into_iter().map(Felt::from).collect())
}
