//! TEE proving backend.
//!
//! Settles state transitions with AMD SEV-SNP attestations: builds a
//! [`BlockAttestation`] over the block range (the same code path as the
//! `tee_generateQuote` RPC), turns it into an `sp1_proof` payload via
//! [`TeeProver`] (a real SP1 Groth16 proof, or a mock journal when the node
//! runs a mock attester), and packages everything as
//! [`PiltoverInput::TeeInput`].

mod mock;
mod prover;

use std::sync::Arc;

use async_trait::async_trait;
use cainome::cairo_serde::ContractAddress as CainomeContractAddress;
use katana_chain_spec::tee::compute_katana_tee_config_hash;
use katana_chain_spec::ChainSpec;
use katana_primitives::block::BlockNumber;
use katana_primitives::settlement::ProofId;
use katana_primitives::utils::transaction::compute_starknet_to_appchain_message_hash;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, HeaderProvider};
use katana_provider::api::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider::ProviderFactory;
use katana_rpc_types::tee::BlockAttestation;
use katana_rpc_types::{L1ToL2Message, L2ToL1Message};
use katana_tee::attestation::build_block_attestation;
use katana_tee::Attester;
use piltover::{MessageToAppchain, MessageToStarknet, PiltoverInput, TEEInput};
pub use prover::{TeeProver, TeeProverError};

use super::ProvingBackend;
use crate::error::SettlementError;

/// TEE proving backend: attest → prove → `TeeInput`.
pub struct TeeBackend<P> {
    provider: P,
    attester: Arc<dyn Attester>,
    prover: TeeProver,
    /// Versioned environment config hash, precomputed from the chain spec.
    katana_tee_config_hash: Felt,
}

impl<P> TeeBackend<P> {
    pub fn new(
        provider: P,
        attester: Arc<dyn Attester>,
        chain_spec: &ChainSpec,
        prover: TeeProver,
    ) -> Self {
        let chain_id: Felt = chain_spec.id().into();
        let fee_token: Felt = chain_spec.fee_contracts().strk.into();
        let katana_tee_config_hash = compute_katana_tee_config_hash(chain_id, fee_token);

        Self { provider, attester, prover, katana_tee_config_hash }
    }
}

#[async_trait]
impl<P> ProvingBackend for TeeBackend<P>
where
    P: ProviderFactory + Send + Sync,
    <P as ProviderFactory>::Provider:
        BlockHashProvider + HeaderProvider + ReceiptProvider + TransactionProvider,
{
    fn name(&self) -> &'static str {
        match self.prover {
            TeeProver::Mock => "tee (mock proofs)",
            TeeProver::Sp1 { .. } => "tee (sp1)",
        }
    }

    fn proof_type(&self) -> &'static str {
        match self.prover {
            TeeProver::Mock => "mock",
            TeeProver::Sp1 { .. } => "sp1",
        }
    }

    async fn prove(
        &self,
        prev_block: Option<BlockNumber>,
        block: BlockNumber,
    ) -> Result<(PiltoverInput, Option<ProofId>), SettlementError> {
        let attestation = {
            let provider = self.provider.provider();
            build_block_attestation(
                &provider,
                &*self.attester,
                prev_block,
                block,
                None,
                self.katana_tee_config_hash,
            )?
        };

        let (sp1_proof, request_id) = self.prover.prove(&attestation).await?;

        // The settlement service persists this opaque id without knowing any SP1 specifics; the
        // prover type is a node-level constant, inferable from config.
        let proof = request_id.map(|id| ProofId::new(id.as_slice().to_vec()));

        Ok((build_tee_input(&attestation, sp1_proof), proof))
    }

    async fn recover(
        &self,
        prev_block: Option<BlockNumber>,
        block: BlockNumber,
        proof: &ProofId,
    ) -> Result<Option<(PiltoverInput, Option<ProofId>)>, SettlementError> {
        // Rebuilding the attestation is local and cheap; only the range-derived fields enter the
        // payload (the quote's fresh hardware signature does not), so pairing a rebuilt
        // attestation with the recovered proof yields the same submittable `TeeInput` the
        // original prove produced — the proof's journal commits to the range-derived
        // commitment, which is deterministic for historical blocks.
        let Some(sp1_proof) = self.prover.recover(proof).await? else { return Ok(None) };

        let attestation = {
            let provider = self.provider.provider();
            build_block_attestation(
                &provider,
                &*self.attester,
                prev_block,
                block,
                None,
                self.katana_tee_config_hash,
            )?
        };

        Ok(Some((build_tee_input(&attestation, sp1_proof), Some(proof.clone()))))
    }
}

impl<P> std::fmt::Debug for TeeBackend<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeeBackend")
            .field("prover", &self.prover)
            .field("katana_tee_config_hash", &self.katana_tee_config_hash)
            .finish_non_exhaustive()
    }
}

/// Builds the `PiltoverInput::TeeInput` payload from an attestation and its proof felts.
fn build_tee_input(attestation: &BlockAttestation, sp1_proof: Vec<Felt>) -> PiltoverInput {
    let l1_to_l2_msg_hashes =
        attestation.l1_to_l2_messages.iter().map(compute_l1_to_l2_msg_hash).collect();

    PiltoverInput::TeeInput(TEEInput {
        sp1_proof,
        prev_state_root: attestation.prev_state_root,
        state_root: attestation.state_root,
        prev_block_hash: attestation.prev_block_hash,
        block_hash: attestation.block_hash,
        prev_block_number: attestation.prev_block_number,
        block_number: attestation.block_number,
        messages_commitment: attestation.messages_commitment,
        messages_to_starknet: messages_to_starknet(&attestation.l2_to_l1_messages),
        messages_to_appchain: messages_to_appchain(&attestation.l1_to_l2_messages),
        l1_to_l2_msg_hashes,
        katana_tee_config_hash: attestation.katana_tee_config_hash,
    })
}

/// Computes the settlement-to-appchain message hash for a Starknet settlement layer.
///
/// Must match the hash Katana's messaging collector stamps into `Receipt::L1Handler` — the
/// attestation's `messages_commitment` is built from those receipt hashes, and Piltover asserts
/// this list re-hashes to the same commitment.
fn compute_l1_to_l2_msg_hash(msg: &L1ToL2Message) -> Felt {
    // calldata = [from_address, ...payload], mirroring the L1HandlerTx calldata layout.
    let mut calldata = Vec::with_capacity(msg.payload.len() + 1);
    calldata.push(msg.from_address);
    calldata.extend_from_slice(&msg.payload);

    compute_starknet_to_appchain_message_hash(
        msg.from_address,
        msg.to_address.into(),
        msg.nonce,
        msg.entry_point_selector,
        &calldata,
    )
}

fn messages_to_starknet(msgs: &[L2ToL1Message]) -> Vec<MessageToStarknet> {
    msgs.iter()
        .map(|m| MessageToStarknet {
            from_address: CainomeContractAddress(m.from_address.into()),
            to_address: CainomeContractAddress(m.to_address),
            payload: m.payload.clone(),
        })
        .collect()
}

fn messages_to_appchain(msgs: &[L1ToL2Message]) -> Vec<MessageToAppchain> {
    msgs.iter()
        .map(|m| MessageToAppchain {
            from_address: CainomeContractAddress(m.from_address),
            to_address: CainomeContractAddress(m.to_address.into()),
            nonce: m.nonce,
            selector: m.entry_point_selector,
            payload: m.payload.clone(),
        })
        .collect()
}
