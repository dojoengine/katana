//! TEE RPC API implementation.

use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_chain_spec::tee::{
    compute_katana_tee_config_hash, KATANA_TEE_APPCHAIN_MODE, KATANA_TEE_REPORT_VERSION,
    KATANA_TEE_SHARDING_MODE,
};
use katana_chain_spec::ChainSpec;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::Tx;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use katana_provider::api::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider::ProviderFactory;
use katana_rpc_api::error::tee::TeeApiError;
use katana_rpc_api::tee::{
    EventProofResponse, TeeApiServer, TeeL1ToL2Message, TeeL2ToL1Message, TeeQuoteResponse,
};
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
        tee_provider: Arc<dyn TeeProvider>,
        fork_block_number: Option<u64>,
        chain_spec: &ChainSpec,
    ) -> Self {
        let chain_id: Felt = chain_spec.id().into();
        let fee_token: Felt = chain_spec.fee_contracts().strk.into();
        let katana_tee_config_hash = compute_katana_tee_config_hash(chain_id, fee_token);
        info!(
            target: "rpc::tee",
            provider_type = tee_provider.provider_type(),
            ?fork_block_number,
            %chain_id,
            %katana_tee_config_hash,
            "TEE API initialized"
        );
        Self { provider_factory, tee_provider, fork_block_number, katana_tee_config_hash }
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
    ) -> RpcResult<TeeQuoteResponse> {
        let katana_tee_config_hash = self.katana_tee_config_hash;

        debug!(
            target: "rpc::tee",
            ?prev_block,
            block,
            %katana_tee_config_hash,
            "Generating TEE attestation quote"
        );

        let provider = self.provider_factory.provider();

        // Get prev block info
        let (prev_block_id, prev_block_hash, prev_state_root) = match prev_block {
            None => (Felt::MAX, Felt::ZERO, Felt::ZERO),

            Some(num) => {
                let hash = provider
                    .block_hash_by_num(num)
                    .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
                    .ok_or_else(|| {
                        TeeApiError::ProviderError(format!("Block hash not found for block {num}"))
                    })?;

                let header = provider
                    .header_by_number(num)
                    .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
                    .ok_or_else(|| {
                        TeeApiError::ProviderError(format!("Header not found for block {num}"))
                    })?;

                (Felt::from(num), hash, header.state_root)
            }
        };

        let block_hash = provider
            .block_hash_by_num(block)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .unwrap();

        let header = provider
            .header_by_number(block)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                TeeApiError::ProviderError(format!("Header not found for block {block}"))
            })?;

        let state_root = header.state_root;
        let events_commitment = header.events_commitment;

        if let Some(fork_block) = self.fork_block_number {
            let report_data = compute_report_data_sharding(
                prev_state_root,
                state_root,
                prev_block_hash,
                block_hash,
                prev_block_id,
                block.into(),
                fork_block.into(),
                events_commitment,
                katana_tee_config_hash,
            );

            let quote = self
                .tee_provider
                .generate_quote(&report_data)
                .map_err(|e| TeeApiError::QuoteGenerationFailed(e.to_string()))?;

            info!(
                target: "rpc::tee",
                ?prev_block_id,
                block_number = block,
                %prev_block_hash,
                %block_hash,
                %katana_tee_config_hash,
                quote_size = quote.len(),
                "Generated TEE attestation quote"
            );

            Ok(TeeQuoteResponse {
                quote: format!("0x{}", hex::encode(&quote)),
                prev_state_root,
                state_root,
                prev_block_hash,
                block_hash,
                prev_block_number: prev_block,
                block_number: block,
                fork_block_number: self.fork_block_number,
                events_commitment,
                katana_tee_config_hash,
                l1_to_l2_messages: Vec::new(),
                l2_to_l1_messages: Vec::new(),
                messages_commitment: Felt::ZERO,
            })
        } else {
            // Gather all L1<->L2 messages from prev_block+1 to block_id (inclusive)
            let start_block = prev_block.map(|n| n + 1).unwrap_or(0);

            let mut l2_to_l1_messages: Vec<TeeL2ToL1Message> = Vec::new();
            let mut l1_to_l2_messages: Vec<TeeL1ToL2Message> = Vec::new();

            let mut l2_to_l1_msg_hashes: Vec<Felt> = Vec::new();
            let mut l1_to_l2_msg_hashes: Vec<Felt> = Vec::new();

            for block_num in start_block..=block {
                let block_id_or_num = BlockHashOrNumber::Num(block_num);

                let receipts = provider
                    .receipts_by_block(block_id_or_num)
                    .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
                    .unwrap_or_default();

                let txs = provider
                    .transactions_by_block(block_id_or_num)
                    .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
                    .unwrap_or_default();

                for receipt in &receipts {
                    // L2->L1: each message in messages_sent
                    for msg in receipt.messages_sent() {
                        let len = Felt::from(msg.payload.len());
                        let payload_hash = Poseidon::hash_array(
                            &std::iter::once(len)
                                .chain(msg.payload.iter().copied())
                                .collect::<Vec<_>>(),
                        );
                        let msg_hash = Poseidon::hash_array(&[
                            msg.from_address.into(),
                            msg.to_address,
                            payload_hash,
                        ]);
                        l2_to_l1_msg_hashes.push(msg_hash);
                        l2_to_l1_messages.push(TeeL2ToL1Message {
                            from_address: msg.from_address.into(),
                            to_address: msg.to_address,
                            payload: msg.payload.clone(),
                        });
                    }

                    // L1->L2: message_hash from L1Handler receipt; full fields from the tx
                    if let Receipt::L1Handler(l1h) = receipt {
                        let felt = Felt::from_bytes_be_slice(&l1h.message_hash.0);
                        l1_to_l2_msg_hashes.push(felt);
                    }
                }

                // Collect full L1→L2 message fields from L1Handler transactions
                for tx in &txs {
                    if let Tx::L1Handler(l1h) = &tx.transaction {
                        // calldata[0] is always the Ethereum sender address as a Felt
                        let from_address = l1h.calldata.first().copied().unwrap_or(Felt::ZERO);
                        let payload = l1h.calldata.get(1..).unwrap_or_default().to_vec();
                        l1_to_l2_messages.push(TeeL1ToL2Message {
                            from_address,
                            to_address: l1h.contract_address.into(),
                            selector: l1h.entry_point_selector,
                            payload,
                            nonce: l1h.nonce,
                        });
                    }
                }
            }

            // Combine both directions into a single messages commitment
            let l2_to_l1_commitment = Poseidon::hash_array(&l2_to_l1_msg_hashes);
            let l1_to_l2_commitment = Poseidon::hash_array(&l1_to_l2_msg_hashes);
            let messages_commitment =
                Poseidon::hash_array(&[l2_to_l1_commitment, l1_to_l2_commitment]);

            let report_data = compute_report_data_appchain(
                prev_state_root,
                state_root,
                prev_block_hash,
                block_hash,
                prev_block_id,
                block.into(),
                messages_commitment,
                katana_tee_config_hash,
            );

            let quote = self
                .tee_provider
                .generate_quote(&report_data)
                .map_err(|e| TeeApiError::QuoteGenerationFailed(e.to_string()))?;

            info!(
                target: "rpc::tee",
                ?prev_block_id,
                block_number = block,
                %prev_block_hash,
                %block_hash,
                %katana_tee_config_hash,
                quote_size = quote.len(),
                l2_to_l1_count = l2_to_l1_messages.len(),
                l1_to_l2_count = l1_to_l2_messages.len(),
                "Generated TEE attestation quote"
            );

            Ok(TeeQuoteResponse {
                quote: format!("0x{}", hex::encode(&quote)),
                prev_state_root,
                state_root,
                prev_block_hash,
                block_hash,
                prev_block_number: prev_block,
                block_number: block,
                fork_block_number: self.fork_block_number,
                events_commitment,
                katana_tee_config_hash,
                l1_to_l2_messages,
                l2_to_l1_messages,
                messages_commitment,
            })
        }
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

/// Compute the 64-byte report data for fork/sharding attestation (v1 schema).
///
/// ```text
/// commitment = Poseidon(
///     KATANA_TEE_REPORT_VERSION,
///     KATANA_TEE_SHARDING_MODE,
///     prev_state_root,
///     state_root,
///     prev_block_hash,
///     block_hash,
///     prev_block_number,
///     block_number,
///     fork_block_number,
///     events_commitment,
///     katana_tee_config_hash,
/// )
/// report_data = commitment_bytes_be ++ katana_tee_config_hash_bytes_be
/// ```
#[allow(clippy::too_many_arguments)]
fn compute_report_data_sharding(
    prev_state_root: Felt,
    state_root: Felt,
    prev_block_hash: Felt,
    block_hash: Felt,
    prev_block_number: Felt,
    block_number: Felt,
    fork_block_number: Felt,
    events_commitment: Felt,
    katana_tee_config_hash: Felt,
) -> [u8; 64] {
    let commitment = Poseidon::hash_array(&[
        KATANA_TEE_REPORT_VERSION.into(),
        KATANA_TEE_SHARDING_MODE.into(),
        prev_state_root,
        state_root,
        prev_block_hash,
        block_hash,
        prev_block_number,
        block_number,
        fork_block_number,
        events_commitment,
        katana_tee_config_hash,
    ]);

    let report_data = encode_report_data(commitment, katana_tee_config_hash);

    debug!(
        target: "rpc::tee",
        %state_root,
        %block_hash,
        ?fork_block_number,
        %events_commitment,
        %katana_tee_config_hash,
        %commitment,
        "Computed report data for attestation"
    );

    report_data
}

/// Compute the 64-byte report data for appchain attestation (v1 schema).
///
/// ```text
/// commitment = Poseidon(
///     KATANA_TEE_REPORT_VERSION,
///     KATANA_TEE_APPCHAIN_MODE,
///     prev_state_root,
///     state_root,
///     prev_block_hash,
///     block_hash,
///     prev_block_number,
///     block_number,
///     messages_commitment,
///     katana_tee_config_hash,
/// )
/// report_data = commitment_bytes_be ++ katana_tee_config_hash_bytes_be
/// ```
#[allow(clippy::too_many_arguments)]
fn compute_report_data_appchain(
    prev_state_root: Felt,
    state_root: Felt,
    prev_block_hash: Felt,
    block_hash: Felt,
    prev_block_number: Felt,
    block_number: Felt,
    messages_commitment: Felt,
    katana_tee_config_hash: Felt,
) -> [u8; 64] {
    let commitment = Poseidon::hash_array(&[
        KATANA_TEE_REPORT_VERSION.into(),
        KATANA_TEE_APPCHAIN_MODE.into(),
        prev_state_root,
        state_root,
        prev_block_hash,
        block_hash,
        prev_block_number,
        block_number,
        messages_commitment,
        katana_tee_config_hash,
    ]);

    let report_data = encode_report_data(commitment, katana_tee_config_hash);

    debug!(
        target: "rpc::tee",
        %state_root,
        %block_hash,
        %katana_tee_config_hash,
        %commitment,
        "Computed report data for attestation"
    );

    report_data
}

fn encode_report_data(commitment: Felt, katana_tee_config_hash: Felt) -> [u8; 64] {
    let mut report_data = [0u8; 64];
    report_data[..32].copy_from_slice(&commitment.to_bytes_be());
    report_data[32..].copy_from_slice(&katana_tee_config_hash.to_bytes_be());
    report_data
}
