//! TEE RPC API implementation.

use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_chain_spec::tee::compute_katana_tee_config_hash;
use katana_chain_spec::ChainSpec;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::hash::{Poseidon, StarkHash};
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use katana_provider::api::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider::ProviderFactory;
use katana_rpc_api::error::tee::TeeApiError;
use katana_rpc_api::tee::TeeApiServer;
use katana_rpc_types::tee::{BlockAttestation, EventProofResponse};
use katana_tee::attestation::{build_block_attestation, AttestationError};
use katana_tee::Attester;
use tracing::{debug, info};

/// TEE API implementation.
#[allow(missing_debug_implementations)]
pub struct TeeApi<PF>
where
    PF: ProviderFactory,
{
    /// Storage provider factory for accessing blockchain state.
    provider_factory: PF,
    /// TEE attester for generating attestation quotes.
    attester: Arc<dyn Attester>,
    /// The block number Katana forked from (if running in fork mode).
    /// Included in report_data so SP1 can prove fork freshness.
    fork_block_number: Option<u64>,
    /// Versioned environment config hash precomputed from the chain spec at
    /// construction time. Bound into every attestation's `report_data`.
    katana_tee_config_hash: Felt,
}

impl<PF> TeeApi<PF>
where
    PF: ProviderFactory,
{
    /// Create a new TEE API instance.
    ///
    /// The versioned environment config hash is derived from the chain spec
    /// at construction time — `pedersen_array([KATANA_TEE_CONFIG_VERSION,
    /// chain_id, fee_token_address])` — and bound into every attestation's
    /// `report_data`.
    pub fn new(
        provider_factory: PF,
        attester: Arc<dyn Attester>,
        fork_block_number: Option<u64>,
        chain_spec: &ChainSpec,
    ) -> Self {
        let chain_id: Felt = chain_spec.id().into();
        let fee_token: Felt = chain_spec.fee_contracts().strk.into();
        let katana_tee_config_hash = compute_katana_tee_config_hash(chain_id, fee_token);
        info!(
            target: "rpc::tee",
            attester = attester.name(),
            ?fork_block_number,
            %chain_id,
            %katana_tee_config_hash,
            "TEE API initialized"
        );
        Self { provider_factory, attester, fork_block_number, katana_tee_config_hash }
    }
}

#[async_trait]
impl<PF> TeeApiServer for TeeApi<PF>
where
    PF: ProviderFactory + Send + Sync + 'static,
    <PF as ProviderFactory>::Provider: BlockHashProvider
        + BlockNumberProvider
        + HeaderProvider
        + ReceiptProvider
        + TransactionProvider
        + Send
        + Sync,
{
    async fn generate_quote(
        &self,
        prev_block: Option<BlockNumber>,
        block: BlockNumber,
    ) -> RpcResult<BlockAttestation> {
        let provider = self.provider_factory.provider();

        let attestation = build_block_attestation(
            &provider,
            &*self.attester,
            prev_block,
            block,
            self.fork_block_number,
            self.katana_tee_config_hash,
        )
        .map_err(|e| match e {
            AttestationError::Quote(e) => TeeApiError::QuoteGenerationFailed(e.to_string()),
            e => TeeApiError::ProviderError(e.to_string()),
        })?;

        Ok(attestation)
    }

    async fn get_event_proof(
        &self,
        block_number: u64,
        event_index: u32,
    ) -> RpcResult<EventProofResponse> {
        debug!(target: "rpc::tee", block_number, event_index, "Generating event inclusion proof");

        let provider = self.provider_factory.provider();
        let block_id = BlockHashOrNumber::Num(block_number);

        // Get block header for events_commitment
        let header = provider
            .header_by_number(block_number)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                TeeApiError::EventProofError(format!("Block {block_number} not found"))
            })?;

        // Get receipts and transactions to reconstruct event hashes
        let receipts = provider
            .receipts_by_block(block_id)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                TeeApiError::EventProofError(format!("No receipts found for block {block_number}"))
            })?;

        let transactions = provider
            .transactions_by_block(block_id)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                TeeApiError::EventProofError(format!(
                    "No transactions found for block {block_number}"
                ))
            })?;

        // Build flattened (tx_hash, event) pairs — same iteration order as
        // compute_event_commitment in backend/mod.rs
        let mut event_hashes = Vec::new();
        let mut event_components: Vec<(Felt, &katana_primitives::receipt::Event)> = Vec::new();

        for (tx, receipt) in transactions.iter().zip(receipts.iter()) {
            for event in receipt.events() {
                let keys_hash = Poseidon::hash_array(&event.keys);
                let data_hash = Poseidon::hash_array(&event.data);
                let event_hash = Poseidon::hash_array(&[
                    tx.hash,
                    event.from_address.into(),
                    keys_hash,
                    data_hash,
                ]);
                event_hashes.push(event_hash);
                event_components.push((tx.hash, event));
            }
        }

        let events_count = event_hashes.len() as u32;

        if event_index >= events_count {
            return Err(TeeApiError::EventProofError(format!(
                "Event index {event_index} out of bounds (block has {events_count} events)"
            ))
            .into());
        }

        // Build the Merkle-Patricia trie and extract proof for the requested event.
        // Uses the same 64-bit key scheme as compute_merkle_root in katana-trie.
        let (computed_root, proof) = katana_trie::compute_merkle_root_with_proof::<Poseidon>(
            &event_hashes,
            event_index as usize,
        )
        .map_err(|e| TeeApiError::EventProofError(e.to_string()))?;

        // Sanity check: computed root must match header's events_commitment
        if computed_root != header.events_commitment {
            return Err(TeeApiError::EventProofError(format!(
                "Computed events root {computed_root:#x} does not match header commitment {:#x}",
                header.events_commitment
            ))
            .into());
        }

        let (tx_hash, event) = event_components[event_index as usize];

        info!(
            target: "rpc::tee",
            block_number,
            event_index,
            events_count,
            proof_nodes = proof.0.len(),
            "Generated event inclusion proof"
        );

        Ok(EventProofResponse {
            block_number,
            events_commitment: header.events_commitment,
            events_count,
            event_hash: event_hashes[event_index as usize],
            event_index,
            merkle_proof: proof.into(),
            tx_hash,
            from_address: event.from_address.into(),
            keys: event.keys.clone(),
            data: event.data.clone(),
        })
    }
}
