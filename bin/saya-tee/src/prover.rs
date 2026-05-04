//! TEE prover — submits a TEE attestation to the SP1 proving service and retrieves the resulting
//! proof.
//!
//! When `mock_prove` is set, the entire SP1 / AMD KDS / cert chain pipeline is
//! skipped and the prover synthesizes a `TeeProof` whose `data` is a raw
//! big-endian felt array encoding a stub `VerifierJournal` (see
//! [`crate::mock_proof`]). The paired `TeePiltoverSettlementBackend` recognises
//! this format and forwards the felts directly to the on-chain
//! `mock_amd_tee_registry` contract instead of going through
//! `OnchainProof::decode_json` and `StarknetCalldata::from_proof`.

use anyhow::Result;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::tee::BlockAttestation;
use katana_tee::amd::ProverConfig;
use saya_core::{
    prover::{HasBlockNumber, PipelineStage, PipelineStageBuilder, TeeProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    tee::TeeAttestation,
};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, info};

use crate::mock_proof;
use crate::prover_impl::TeeAttestation as TeeAttestationProver;

/// Submits a [`TeeAttestation`] to the TEE proving service and emits the resulting [`TeeProof`].
#[derive(Debug)]
pub struct TeeProver {
    provider_url: String,
    registry_address: ContractAddress,
    private_key: String,
    /// When `true`, skip the real KDS/cert/SP1 pipeline and synthesize a stub
    /// `VerifierJournal` for the paired `mock_amd_tee_registry` contract.
    mock_prove: bool,
    input_channel: Receiver<TeeAttestation>,
    output_channel: Sender<TeeProof>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeProverBuilder {
    provider_url: String,
    registry_address: ContractAddress,
    private_key: String,
    mock_prove: bool,
    input_channel: Option<Receiver<TeeAttestation>>,
    output_channel: Option<Sender<TeeProof>>,
}

impl TeeProverBuilder {
    pub fn new(
        provider_url: String,
        registry_address: ContractAddress,
        private_key: String,
        mock_prove: bool,
    ) -> Self {
        Self {
            provider_url,
            registry_address,
            private_key,
            mock_prove,
            input_channel: None,
            output_channel: None,
        }
    }
}

impl PipelineStageBuilder for TeeProverBuilder {
    type Stage = TeeProver;

    fn build(self) -> Result<Self::Stage> {
        Ok(TeeProver {
            provider_url: self.provider_url,
            registry_address: self.registry_address,
            private_key: self.private_key,
            mock_prove: self.mock_prove,
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn input_channel(mut self, input_channel: Receiver<TeeAttestation>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<TeeProof>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl PipelineStage for TeeProver {
    type Input = TeeAttestation;
    type Output = TeeProof;
}

impl Daemon for TeeProver {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl TeeProver {
    async fn run(mut self) {
        loop {
            let attestation = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                t = self.input_channel.recv() => match t {
                    Some(t) => t,
                    None => break,
                },
            };

            debug!(
                block_number = attestation.block_number(),
                "Submitting TEE attestation to prover"
            );

            let proof = match self.prove(attestation).await {
                Ok(p) => p,
                Err(e) => {
                    error!("TEE proof generation failed: {}", e);
                    continue;
                }
            };

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.output_channel.send(proof) => {},
            }
        }

        debug!("TeeProver graceful shutdown finished");
        self.finish_handle.finish();
    }

    async fn prove(&self, attestation: TeeAttestation) -> Result<TeeProof> {
        let block_number = attestation.block_number();

        let proof_raw = if self.mock_prove {
            info!(block_number, "TEE mock proving (no SP1, no KDS, no AMD verification)");

            let commitment = mock_proof::compute_appchain_commitment(
                attestation.prev_state_root,
                attestation.state_root,
                attestation.prev_block_hash,
                attestation.block_hash,
                attestation.prev_block_number,
                attestation.block_number,
                attestation.messages_commitment,
            );

            let felts = mock_proof::serialize_mock_journal(commitment);
            mock_proof::felts_to_bytes(&felts)
        } else {
            info!(block_number, "TEE proving started for block batch");

            let response = BlockAttestation {
                quote: attestation.quote.clone(),
                prev_state_root: attestation.prev_state_root.clone(),
                state_root: attestation.state_root.clone(),
                prev_block_hash: attestation.prev_block_hash.clone(),
                block_hash: attestation.block_hash.clone(),
                prev_block_number: Some(attestation.prev_block_number),
                block_number: attestation.block_number,
                messages_commitment: attestation.messages_commitment,
                l2_to_l1_messages: Vec::new(),
                l1_to_l2_messages: Vec::new(),
                events_commitment: Felt::ZERO,
                fork_block_number: None,
            };

            let tee = TeeAttestationProver::from_response(&response)?;
            let config = ProverConfig {
                rpc_url: None,
                private_key: Some(self.private_key.clone()),
                skip_time_validity_check: false,
            };
            let proof =
                tee.generate_proof(&self.provider_url, self.registry_address, config).await?;
            let proof_raw = proof.encode_json()?;

            info!(block_number, "TEE proving completed, proof size: {} bytes", proof_raw.len());

            proof_raw
        };

        Ok(TeeProof {
            blocks: attestation.blocks,
            data: proof_raw,
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number: attestation.prev_block_number,
            block_number: attestation.block_number,
            messages_commitment: attestation.messages_commitment,
            l2_to_l1_messages: attestation.l2_to_l1_messages,
            l1_to_l2_messages: attestation.l1_to_l2_messages,
        })
    }
}
