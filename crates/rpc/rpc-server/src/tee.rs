//! TEE RPC API implementation.

use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use katana_provider::api::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider::ProviderFactory;
use katana_rpc_api::error::tee::TeeApiError;
use katana_rpc_api::tee::{EventProofResponse, TeeApiServer, TeeQuoteResponse};
use katana_tee::TeeProvider;
use starknet_types_core::hash::{Poseidon, StarkHash};
use tracing::{debug, info};

/// TEE API implementation.
#[allow(missing_debug_implementations)]
pub struct TeeApi<PF>
where
    PF: ProviderFactory,
{
    /// Storage provider factory for accessing blockchain state.
    provider_factory: PF,
    /// TEE provider for generating attestation quotes.
    tee_provider: Arc<dyn TeeProvider>,
    /// The block number Katana forked from (if running in fork mode).
    /// Included in report_data so SP1 can prove fork freshness.
    fork_block_number: Option<u64>,
}

impl<PF> TeeApi<PF>
where
    PF: ProviderFactory,
{
    /// Create a new TEE API instance.
    pub fn new(
        provider_factory: PF,
        tee_provider: Arc<dyn TeeProvider>,
        fork_block_number: Option<u64>,
    ) -> Self {
        info!(
            target: "rpc::tee",
            provider_type = tee_provider.provider_type(),
            ?fork_block_number,
            "TEE API initialized"
        );
        Self { provider_factory, tee_provider, fork_block_number }
    }

    /// Compute the 64-byte report data for attestation.
    /// report_data = Poseidon(state_root, block_hash, fork_block_number, events_commitment)
    fn compute_report_data(
        &self,
        state_root: Felt,
        block_hash: Felt,
        events_commitment: Felt,
    ) -> [u8; 64] {
        let fb = Felt::from(self.fork_block_number.unwrap_or(0));
        let commitment = Poseidon::hash_array(&[state_root, block_hash, fb, events_commitment]);

        // Convert Felt to bytes (32 bytes) and pad to 64 bytes
        let commitment_bytes = commitment.to_bytes_be();

        let mut report_data = [0u8; 64];
        // Place the 32-byte hash in the first half
        report_data[..32].copy_from_slice(&commitment_bytes);
        // Second half remains zeros

        debug!(
            target: "rpc::tee",
            %state_root,
            %block_hash,
            fork_block_number = ?self.fork_block_number,
            %events_commitment,
            %commitment,
            "Computed report data for attestation"
        );

        report_data
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
        prev_block_id: Option<BlockNumber>,
        block_id: BlockNumber,
    ) -> RpcResult<TeeQuoteResponse> {
        debug!(
            target: "rpc::tee",
            ?prev_block_id,
            block_id,
            "Generating TEE attestation quote"
        );

        // Get blockchain state for the requested block(s)
        let provider = self.provider_factory.provider();

        let (prev_block_number, prev_block_hash, prev_state_root) = match prev_block_id {
            Some(prev_num) => {
                let prev_hash = provider
                    .block_hash_by_num(prev_num)
                    .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
                    .ok_or_else(|| {
                        TeeApiError::ProviderError(format!(
                            "Block hash not found for block {prev_num}"
                        ))
                    })?;
                let prev_header = provider
                    .header_by_number(prev_num)
                    .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
                    .ok_or_else(|| {
                        TeeApiError::ProviderError(format!("Header not found for block {prev_num}"))
                    })?;
                (prev_num, prev_hash, prev_header.state_root)
            }
            None => (u64::MAX, Felt::ZERO, Felt::ZERO),
        };

        let block_hash = provider
            .block_hash_by_num(block_id)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                TeeApiError::ProviderError(format!("Block hash not found for block {block_id}"))
            })?;

        // Get the header to retrieve state_root
        let header = provider
            .header_by_number(block_id)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                TeeApiError::ProviderError(format!("Header not found for block {block_id}"))
            })?;

        let state_root = header.state_root;
        let events_commitment = header.events_commitment;

        // Compute report data: Poseidon(state_root, block_hash, fork_block, events_commitment)
        let report_data = self.compute_report_data(state_root, block_hash, events_commitment);

        // Generate the attestation quote
        let quote = self
            .tee_provider
            .generate_quote(&report_data)
            .map_err(|e| TeeApiError::QuoteGenerationFailed(e.to_string()))?;

        info!(
            target: "rpc::tee",
            prev_block_number,
            block_number = block_id,
            %prev_block_hash,
            %block_hash,
            quote_size = quote.len(),
            "Generated TEE attestation quote"
        );

        Ok(TeeQuoteResponse {
            quote: format!("0x{}", hex::encode(&quote)),
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number,
            block_number: block_id,
            fork_block_number: self.fork_block_number,
            events_commitment,
        })
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
