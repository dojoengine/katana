//! TEE RPC API implementation.

use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::Tx;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use katana_provider::api::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider::ProviderFactory;
use katana_rpc_api::error::tee::TeeApiError;
use katana_rpc_api::tee::{TeeApiServer, TeeL1ToL2Message, TeeL2ToL1Message, TeeQuoteResponse};
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
}

impl<PF> TeeApi<PF>
where
    PF: ProviderFactory,
{
    /// Create a new TEE API instance.
    pub fn new(provider_factory: PF, tee_provider: Arc<dyn TeeProvider>) -> Self {
        info!(
            target: "rpc::tee",
            provider_type = tee_provider.provider_type(),
            "TEE API initialized"
        );
        Self { provider_factory, tee_provider }
    }

    /// Compute the 64-byte report data for attestation.
    ///
    /// The report data is: Poseidon(prev_state_root, state_root, prev_block_hash, block_hash,
    /// prev_block_number, block_number, messages_commitment) padded to 64 bytes.
    fn compute_report_data(
        &self,
        prev_state_root: Felt,
        state_root: Felt,
        prev_block_hash: Felt,
        block_hash: Felt,
        prev_block_number: Felt,
        block_number: Felt,
        messages_commitment: Felt,
    ) -> [u8; 64] {
        let commitment = Poseidon::hash_array(&[
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number,
            block_number,
            messages_commitment,
        ]);

        let commitment_bytes = commitment.to_bytes_be();

        let mut report_data = [0u8; 64];
        report_data[..32].copy_from_slice(&commitment_bytes);

        debug!(
            target: "rpc::tee",
            %state_root,
            %block_hash,
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
        debug!(target: "rpc::tee", "Generating TEE attestation quote");

        let provider = self.provider_factory.provider();

        // Get prev block info
        let (prev_block_number, prev_block_hash, prev_state_root) = match prev_block_id {
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
            None => (Felt::MAX, Felt::ZERO, Felt::ZERO),
        };

        let block_hash = provider
            .block_hash_by_num(block_id)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .unwrap();

        let header = provider
            .header_by_number(block_id)
            .map_err(|e| TeeApiError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                TeeApiError::ProviderError(format!("Header not found for block {block_id}"))
            })?;

        let state_root = header.state_root;

        // Gather all L1<->L2 messages from prev_block+1 to block_id (inclusive)
        let start_block = prev_block_id.map(|n| n + 1).unwrap_or(0);

        let mut l2_to_l1_messages: Vec<TeeL2ToL1Message> = Vec::new();
        let mut l1_to_l2_messages: Vec<TeeL1ToL2Message> = Vec::new();

        let mut l2_to_l1_msg_hashes: Vec<Felt> = Vec::new();
        let mut l1_to_l2_msg_hashes: Vec<Felt> = Vec::new();

        for block_num in start_block..=block_id {
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
                    let from_address =
                        l1h.calldata.first().copied().unwrap_or(Felt::ZERO);
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

        let report_data = self.compute_report_data(
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number,
            Felt::from(block_id),
            messages_commitment,
        );

        let quote = self
            .tee_provider
            .generate_quote(&report_data)
            .map_err(|e| TeeApiError::QuoteGenerationFailed(e.to_string()))?;

        info!(
            target: "rpc::tee",
            block_number = block_id,
            %block_hash,
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
            prev_block_number,
            block_number: Felt::from(block_id),
            messages_commitment,
            l2_to_l1_messages,
            l1_to_l2_messages,
        })
    }
}
