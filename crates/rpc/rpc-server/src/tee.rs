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
        debug!(
            target: "rpc::tee",
            ?prev_block,
            block,
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

/// Compute the 64-byte report data for attestation.
///
/// ```text
/// Poseidon(
///     prev_state_root,
///     state_root,
///     prev_block_hash,
///     block_hash,
///     prev_block_number,
///     block_number,
///     fork_block_number,
///     events_commitment,
/// )
/// report_data = commitment_bytes_be ++ [0u8; 32]   // 64 bytes total
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
) -> [u8; 64] {
    // Compute Poseidon hash of state_root and block_hash
    let commitment = Poseidon::hash_array(&[
        prev_state_root,
        state_root,
        prev_block_hash,
        block_hash,
        prev_block_number,
        block_number,
        fork_block_number,
        events_commitment,
    ]);

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
        ?fork_block_number,
        %events_commitment,
        %commitment,
        "Computed report data for attestation"
    );

    report_data
}

/// Compute the 64-byte report data for attestation.
///
/// ```text
/// Poseidon(
///     prev_state_root,
///     state_root,
///     prev_block_hash,
///     block_hash,
///     prev_block_number,
///     block_number,
///     messages_commitment
/// )
/// report_data = commitment_bytes_be ++ [0u8; 32]   // 64 bytes total
/// ```
fn compute_report_data_appchain(
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use katana_primitives::block::{
        Block, BlockHash, BlockHashOrNumber, BlockNumber, FinalityStatus, Header,
    };
    use katana_primitives::contract::ContractAddress;
    use katana_primitives::execution::{
        CallEntryPointVariant, CallInfo, TransactionExecutionInfo, TypedTransactionExecutionInfo,
    };
    use katana_primitives::fee::FeeInfo;
    use katana_primitives::receipt::{Event, InvokeTxReceipt, Receipt};
    use katana_primitives::transaction::{InvokeTx, Tx, TxHash, TxNumber, TxWithHash};
    use katana_primitives::{address, Felt};
    use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, BlockWriter};
    use katana_provider::{
        DbProviderFactory, MutableProvider, ProviderError, ProviderFactory, ProviderResult,
    };
    use katana_tee::TeeError;
    use katana_trie::compute_merkle_root;
    use rstest::rstest;
    use starknet::macros::felt;
    use starknet_types_core::hash::StarkHash;

    use super::*;

    // JSON-RPC error codes exposed by `TeeApiError`. Mirrored here because the
    // constants in `katana_rpc_api::error::tee` are module-private.
    const TEE_PROVIDER_ERROR_CODE: i32 = 102;
    const TEE_EVENT_PROOF_ERROR_CODE: i32 = 103;

    // --- Test doubles ---

    /// TEE provider stub. `get_event_proof` never calls `generate_quote`.
    #[derive(Debug)]
    struct StubTeeProvider;

    impl TeeProvider for StubTeeProvider {
        fn generate_quote(&self, _: &[u8; 64]) -> Result<Vec<u8>, TeeError> {
            unreachable!("get_event_proof does not call generate_quote")
        }
        fn provider_type(&self) -> &'static str {
            "Stub"
        }
    }

    /// Shared state behind the fake provider. Each slot is consumed on first
    /// call; unused trait methods panic with `unimplemented!`.
    #[derive(Debug, Default)]
    struct FakeState {
        header: Mutex<Option<ProviderResult<Option<Header>>>>,
        receipts: Mutex<Option<ProviderResult<Option<Vec<Receipt>>>>>,
        transactions: Mutex<Option<ProviderResult<Option<Vec<TxWithHash>>>>>,
    }

    /// Handle into `FakeState`. A newtype (not `Arc<FakeState>`) so we can
    /// legally `impl MutableProvider` for it under Rust's orphan rules.
    #[derive(Debug, Default, Clone)]
    struct FakeProvider(Arc<FakeState>);

    impl FakeProvider {
        fn with_header(self, result: ProviderResult<Option<Header>>) -> Self {
            *self.0.header.lock().unwrap() = Some(result);
            self
        }
        fn with_receipts(self, result: ProviderResult<Option<Vec<Receipt>>>) -> Self {
            *self.0.receipts.lock().unwrap() = Some(result);
            self
        }
        fn with_transactions(self, result: ProviderResult<Option<Vec<TxWithHash>>>) -> Self {
            *self.0.transactions.lock().unwrap() = Some(result);
            self
        }
    }

    impl HeaderProvider for FakeProvider {
        fn header(&self, _: BlockHashOrNumber) -> ProviderResult<Option<Header>> {
            self.0.header.lock().unwrap().take().expect("header not configured")
        }
    }

    impl ReceiptProvider for FakeProvider {
        fn receipt_by_hash(&self, _: TxHash) -> ProviderResult<Option<Receipt>> {
            unimplemented!()
        }
        fn receipts_by_block(&self, _: BlockHashOrNumber) -> ProviderResult<Option<Vec<Receipt>>> {
            self.0.receipts.lock().unwrap().take().expect("receipts not configured")
        }
    }

    impl TransactionProvider for FakeProvider {
        fn transaction_by_hash(&self, _: TxHash) -> ProviderResult<Option<TxWithHash>> {
            unimplemented!()
        }
        fn transactions_by_block(
            &self,
            _: BlockHashOrNumber,
        ) -> ProviderResult<Option<Vec<TxWithHash>>> {
            self.0.transactions.lock().unwrap().take().expect("transactions not configured")
        }
        fn transaction_by_block_and_idx(
            &self,
            _: BlockHashOrNumber,
            _: u64,
        ) -> ProviderResult<Option<TxWithHash>> {
            unimplemented!()
        }
        fn transaction_count_by_block(&self, _: BlockHashOrNumber) -> ProviderResult<Option<u64>> {
            unimplemented!()
        }
        fn transaction_block_num_and_hash(
            &self,
            _: TxHash,
        ) -> ProviderResult<Option<(BlockNumber, BlockHash)>> {
            unimplemented!()
        }
        fn transaction_in_range(
            &self,
            _: std::ops::Range<TxNumber>,
        ) -> ProviderResult<Vec<TxWithHash>> {
            unimplemented!()
        }
    }

    impl BlockHashProvider for FakeProvider {
        fn latest_hash(&self) -> ProviderResult<BlockHash> {
            unimplemented!()
        }
        fn block_hash_by_num(&self, _: BlockNumber) -> ProviderResult<Option<BlockHash>> {
            unimplemented!()
        }
    }

    impl BlockNumberProvider for FakeProvider {
        fn latest_number(&self) -> ProviderResult<BlockNumber> {
            unimplemented!()
        }
        fn block_number_by_hash(&self, _: BlockHash) -> ProviderResult<Option<BlockNumber>> {
            unimplemented!()
        }
    }

    impl MutableProvider for FakeProvider {
        fn commit(self) -> ProviderResult<()> {
            unreachable!("FakeProvider commit is not used by get_event_proof")
        }
    }

    #[derive(Debug, Default, Clone)]
    struct FakeFactory(FakeProvider);

    impl FakeFactory {
        fn new(provider: FakeProvider) -> Self {
            Self(provider)
        }
    }

    impl ProviderFactory for FakeFactory {
        type Provider = FakeProvider;
        type ProviderMut = FakeProvider;

        fn provider(&self) -> Self::Provider {
            self.0.clone()
        }
        fn provider_mut(&self) -> Self::ProviderMut {
            self.0.clone()
        }
    }

    // --- Helpers ---

    fn build_api<PF>(factory: PF) -> TeeApi<PF>
    where
        PF: ProviderFactory,
    {
        TeeApi::new(factory, Arc::new(StubTeeProvider), None)
    }

    fn make_tx(hash: Felt) -> TxWithHash {
        TxWithHash { hash, transaction: Tx::Invoke(InvokeTx::V1(Default::default())) }
    }

    fn make_event(from: ContractAddress, keys: Vec<Felt>, data: Vec<Felt>) -> Event {
        Event { from_address: from, keys, data }
    }

    fn make_invoke_receipt(events: Vec<Event>) -> Receipt {
        Receipt::Invoke(InvokeTxReceipt {
            fee: FeeInfo::default(),
            events,
            messages_sent: Vec::new(),
            revert_error: None,
            execution_resources: Default::default(),
        })
    }

    /// Recomputes `events_commitment` the same way tee.rs:320-333 does.
    fn compute_events_commitment(txs: &[TxWithHash], receipts: &[Receipt]) -> Felt {
        let mut event_hashes = Vec::new();
        for (tx, receipt) in txs.iter().zip(receipts.iter()) {
            for event in receipt.events() {
                let keys_hash = Poseidon::hash_array(&event.keys);
                let data_hash = Poseidon::hash_array(&event.data);
                event_hashes.push(Poseidon::hash_array(&[
                    tx.hash,
                    event.from_address.into(),
                    keys_hash,
                    data_hash,
                ]));
            }
        }
        compute_merkle_root::<Poseidon>(&event_hashes).unwrap()
    }

    /// Dummy executions sized to match `txs` — `insert_block_with_states_and_receipts`
    /// expects one entry per transaction.
    fn dummy_executions(count: usize) -> Vec<TypedTransactionExecutionInfo> {
        (0..count)
            .map(|_| {
                TypedTransactionExecutionInfo::Invoke(TransactionExecutionInfo {
                    revert_error: None,
                    execute_call_info: Some(CallInfo {
                        call: CallEntryPointVariant {
                            class_hash: Some(Default::default()),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            })
            .collect()
    }

    /// Seeds a fresh in-memory provider with a single block whose header carries
    /// the correct `events_commitment` for the supplied txs + receipts.
    fn seed_block(
        block_number: BlockNumber,
        txs: Vec<TxWithHash>,
        receipts: Vec<Receipt>,
    ) -> DbProviderFactory {
        let factory = DbProviderFactory::new_in_memory();
        let events_commitment = compute_events_commitment(&txs, &receipts);

        let header = Header { number: block_number, events_commitment, ..Default::default() };
        let executions = dummy_executions(txs.len());
        let sealed = Block { header, body: txs }
            .seal_with_hash_and_status(Felt::from(block_number + 1), FinalityStatus::AcceptedOnL2);

        let provider_mut = factory.provider_mut();
        provider_mut
            .insert_block_with_states_and_receipts(sealed, Default::default(), receipts, executions)
            .unwrap();
        provider_mut.commit().unwrap();
        factory
    }

    // --- Error-path tests (one per early-return branch) ---

    #[tokio::test]
    async fn header_provider_error_maps_to_provider_error() {
        let factory = FakeFactory::new(
            FakeProvider::default().with_header(Err(ProviderError::Other("boom".into()))),
        );
        let err = build_api(factory).get_event_proof(0, 0).await.unwrap_err();
        assert_eq!(err.code(), TEE_PROVIDER_ERROR_CODE);
        assert!(err.message().contains("boom"), "message was: {}", err.message());
    }

    #[tokio::test]
    async fn missing_header_maps_to_event_proof_error() {
        let factory = FakeFactory::new(FakeProvider::default().with_header(Ok(None)));
        let err = build_api(factory).get_event_proof(42, 0).await.unwrap_err();
        assert_eq!(err.code(), TEE_EVENT_PROOF_ERROR_CODE);
        assert!(err.message().contains("Block 42 not found"), "message was: {}", err.message());
    }

    #[tokio::test]
    async fn receipts_provider_error_maps_to_provider_error() {
        let factory = FakeFactory::new(
            FakeProvider::default()
                .with_header(Ok(Some(Header::default())))
                .with_receipts(Err(ProviderError::Other("receipt boom".into()))),
        );
        let err = build_api(factory).get_event_proof(0, 0).await.unwrap_err();
        assert_eq!(err.code(), TEE_PROVIDER_ERROR_CODE);
        assert!(err.message().contains("receipt boom"), "message was: {}", err.message());
    }

    #[tokio::test]
    async fn missing_receipts_maps_to_event_proof_error() {
        let factory = FakeFactory::new(
            FakeProvider::default()
                .with_header(Ok(Some(Header::default())))
                .with_receipts(Ok(None)),
        );
        let err = build_api(factory).get_event_proof(7, 0).await.unwrap_err();
        assert_eq!(err.code(), TEE_EVENT_PROOF_ERROR_CODE);
        assert!(
            err.message().contains("No receipts found for block 7"),
            "message was: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn transactions_provider_error_maps_to_provider_error() {
        let factory = FakeFactory::new(
            FakeProvider::default()
                .with_header(Ok(Some(Header::default())))
                .with_receipts(Ok(Some(Vec::new())))
                .with_transactions(Err(ProviderError::Other("tx boom".into()))),
        );
        let err = build_api(factory).get_event_proof(0, 0).await.unwrap_err();
        assert_eq!(err.code(), TEE_PROVIDER_ERROR_CODE);
        assert!(err.message().contains("tx boom"), "message was: {}", err.message());
    }

    #[tokio::test]
    async fn missing_transactions_maps_to_event_proof_error() {
        let factory = FakeFactory::new(
            FakeProvider::default()
                .with_header(Ok(Some(Header::default())))
                .with_receipts(Ok(Some(Vec::new())))
                .with_transactions(Ok(None)),
        );
        let err = build_api(factory).get_event_proof(9, 0).await.unwrap_err();
        assert_eq!(err.code(), TEE_EVENT_PROOF_ERROR_CODE);
        assert!(
            err.message().contains("No transactions found for block 9"),
            "message was: {}",
            err.message()
        );
    }

    /// Covers the bounds check for both an empty-events block (any index) and
    /// a non-empty block asked for an index past the last event.
    #[rstest]
    #[case::empty_block(vec![], vec![], 0)]
    #[case::past_last(
        vec![make_tx(felt!("0xaa"))],
        vec![make_invoke_receipt(vec![make_event(address!("0x1"), vec![felt!("0x1")], vec![])])],
        5,
    )]
    #[tokio::test]
    async fn event_index_out_of_bounds(
        #[case] txs: Vec<TxWithHash>,
        #[case] receipts: Vec<Receipt>,
        #[case] event_index: u32,
    ) {
        let factory = seed_block(0, txs, receipts);
        let err = build_api(factory).get_event_proof(0, event_index).await.unwrap_err();
        assert_eq!(err.code(), TEE_EVENT_PROOF_ERROR_CODE);
        assert!(err.message().contains("out of bounds"), "message was: {}", err.message());
    }

    // --- Success path ---

    /// Round-trip proof for an interior event that sits on a receipt boundary:
    /// receipt[1] has no events, so the picked index (2) maps to receipt[2]'s
    /// first event, exercising the zip/flatten ordering. Also includes an event
    /// with empty keys/data to exercise `Poseidon::hash_array(&[])`.
    #[tokio::test]
    async fn get_event_proof_round_trip() {
        let tx0 = make_tx(felt!("0xaaaa"));
        let tx1 = make_tx(felt!("0xbbbb"));
        let tx2 = make_tx(felt!("0xcccc"));

        let addr_a = address!("0x111");
        let addr_b = address!("0x222");

        let receipt0 = make_invoke_receipt(vec![
            // Event with empty keys + empty data.
            make_event(addr_a, vec![], vec![]),
            make_event(addr_a, vec![felt!("0x1")], vec![felt!("0xdead")]),
        ]);
        let receipt1 = make_invoke_receipt(vec![]);
        let receipt2 = make_invoke_receipt(vec![
            // Event we'll pick (index 2 overall).
            make_event(addr_b, vec![felt!("0x42")], vec![felt!("0xbeef"), felt!("0xcafe")]),
            make_event(addr_b, vec![felt!("0x43")], vec![felt!("0xf00d")]),
        ]);

        let txs = vec![tx0.clone(), tx1, tx2.clone()];
        let receipts = vec![receipt0, receipt1, receipt2];
        let expected_commitment = compute_events_commitment(&txs, &receipts);
        let factory = seed_block(3, txs, receipts.clone());

        let response = build_api(factory).get_event_proof(3, 2).await.unwrap();

        assert_eq!(response.block_number, 3);
        assert_eq!(response.event_index, 2);
        assert_eq!(response.events_count, 4);
        assert_eq!(response.events_commitment, expected_commitment);
        assert_eq!(response.tx_hash, tx2.hash);

        // Picked event fields match receipts[2].events[0].
        let picked = &receipts[2].events()[0];
        assert_eq!(response.from_address, picked.from_address.into());
        assert_eq!(response.keys, picked.keys);
        assert_eq!(response.data, picked.data);

        // Recomputing the event hash externally must match what the method returned.
        let expected_event_hash = Poseidon::hash_array(&[
            tx2.hash,
            picked.from_address.into(),
            Poseidon::hash_array(&picked.keys),
            Poseidon::hash_array(&picked.data),
        ]);
        assert_eq!(response.event_hash, expected_event_hash);

        // Proof must be non-trivial; the method's internal invariant check
        // (computed_root == header.events_commitment) guarantees it verifies.
        assert!(!response.merkle_proof.0.is_empty(), "merkle proof should be non-empty");
    }
}
