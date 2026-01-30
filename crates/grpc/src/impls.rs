//! Implementations of gRPC traits for katana types.

use katana_pool::TransactionPool;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_server::starknet::{PendingBlockProvider, StarknetApi};

use crate::handlers::StarknetApiProvider;

#[tonic::async_trait]
impl<Pool, PP, PF> StarknetApiProvider for StarknetApi<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    fn spec_version(&self) -> &'static str {
        katana_rpc_api::version::STARKNET_SPEC_VERSION
    }

    fn chain_id(&self) -> Felt {
        StarknetApi::chain_id(self)
    }

    async fn block_number(
        &self,
    ) -> Result<katana_rpc_types::block::BlockNumberResponse, StarknetApiError> {
        // Use the on_io_blocking_task approach like the JSON-RPC implementation
        self.on_io_blocking_task(move |this| {
            let block_number = this.storage().provider().latest_number()?;
            Ok(katana_rpc_types::block::BlockNumberResponse { block_number })
        })
        .await?
    }

    async fn block_hash_and_number(
        &self,
    ) -> Result<katana_rpc_types::block::BlockHashAndNumberResponse, StarknetApiError> {
        self.on_io_blocking_task(move |this| {
            let provider = this.storage().provider();
            let hash = provider.latest_hash()?;
            let number = provider.latest_number()?;
            Ok(katana_rpc_types::block::BlockHashAndNumberResponse::new(hash, number))
        })
        .await?
    }

    async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<katana_rpc_types::block::GetBlockWithTxHashesResponse, StarknetApiError> {
        self.block_with_tx_hashes(block_id).await
    }

    async fn get_block_with_txs(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<katana_rpc_types::block::MaybePreConfirmedBlock, StarknetApiError> {
        self.block_with_txs(block_id).await
    }

    async fn get_block_with_receipts(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<katana_rpc_types::block::GetBlockWithReceiptsResponse, StarknetApiError> {
        self.on_io_blocking_task(move |this| {
            use katana_provider::api::block::BlockIdReader;

            let provider = this.storage().provider();

            if BlockIdOrTag::PreConfirmed == block_id {
                if let Some(block) =
                    katana_rpc_server::starknet::PendingBlockProvider::get_pending_block_with_receipts(&this)?
                {
                    return Ok(katana_rpc_types::block::GetBlockWithReceiptsResponse::PreConfirmed(block));
                }
            }

            if let Some(num) = provider.convert_block_id(block_id)? {
                let block = katana_rpc_types_builder::BlockBuilder::new(num.into(), provider)
                    .build_with_receipts()?
                    .map(katana_rpc_types::block::GetBlockWithReceiptsResponse::Block);

                if let Some(block) = block {
                    Ok(block)
                } else {
                    Err(StarknetApiError::BlockNotFound)
                }
            } else {
                Err(StarknetApiError::BlockNotFound)
            }
        })
        .await?
    }

    async fn get_state_update(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<katana_rpc_types::state_update::StateUpdate, StarknetApiError> {
        self.state_update(block_id).await
    }

    async fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: Felt,
        block_id: BlockIdOrTag,
    ) -> Result<Felt, StarknetApiError> {
        self.on_io_blocking_task(move |this| this.storage_at(contract_address, key, block_id))
            .await?
    }

    async fn get_transaction_status(
        &self,
        transaction_hash: Felt,
    ) -> Result<katana_rpc_types::TxStatus, StarknetApiError> {
        self.on_io_blocking_task(move |this| {
            use katana_primitives::block::FinalityStatus;
            use katana_provider::api::transaction::{ReceiptProvider, TransactionStatusProvider};

            let provider = this.storage().provider();
            let status = provider.transaction_status(transaction_hash)?;

            if let Some(status) = status {
                let Some(receipt) = provider.receipt_by_hash(transaction_hash)? else {
                    return Err(StarknetApiError::unexpected(
                        "Transaction hash exist, but the receipt is missing",
                    ));
                };

                let exec_status = if let Some(reason) = receipt.revert_reason() {
                    katana_rpc_types::ExecutionResult::Reverted { reason: reason.to_string() }
                } else {
                    katana_rpc_types::ExecutionResult::Succeeded
                };

                let status = match status {
                    FinalityStatus::AcceptedOnL1 => {
                        katana_rpc_types::TxStatus::AcceptedOnL1(exec_status)
                    }
                    FinalityStatus::AcceptedOnL2 => {
                        katana_rpc_types::TxStatus::AcceptedOnL2(exec_status)
                    }
                    FinalityStatus::PreConfirmed => {
                        katana_rpc_types::TxStatus::PreConfirmed(exec_status)
                    }
                };

                return Ok(status);
            }

            // Search in the pending block if the transaction is not found
            if let Some(receipt) =
                katana_rpc_server::starknet::PendingBlockProvider::get_pending_receipt(
                    &this,
                    transaction_hash,
                )?
            {
                Ok(katana_rpc_types::TxStatus::PreConfirmed(
                    receipt.receipt.execution_result().clone(),
                ))
            } else {
                // Check if it's in the pool
                use katana_pool::TransactionPool;
                let _ = this.pool().get(transaction_hash).ok_or(StarknetApiError::TxnHashNotFound)?;
                Ok(katana_rpc_types::TxStatus::Received)
            }
        })
        .await?
    }

    async fn get_transaction_by_hash(
        &self,
        transaction_hash: Felt,
    ) -> Result<katana_rpc_types::transaction::RpcTxWithHash, StarknetApiError> {
        self.on_io_blocking_task(move |this| {
            use katana_provider::api::transaction::TransactionProvider;

            if let Some(tx) =
                katana_rpc_server::starknet::PendingBlockProvider::get_pending_transaction(
                    &this,
                    transaction_hash,
                )?
            {
                return Ok(tx);
            }

            let tx = this
                .storage()
                .provider()
                .transaction_by_hash(transaction_hash)?
                .map(katana_rpc_types::transaction::RpcTxWithHash::from);

            tx.ok_or(StarknetApiError::TxnHashNotFound)
        })
        .await?
    }

    async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        index: u64,
    ) -> Result<katana_rpc_types::transaction::RpcTxWithHash, StarknetApiError> {
        self.on_io_blocking_task(move |this| {
            use katana_primitives::block::BlockHashOrNumber;
            use katana_provider::api::block::BlockIdReader;
            use katana_provider::api::transaction::TransactionProvider;

            if BlockIdOrTag::PreConfirmed == block_id {
                if let Some(tx) =
                    katana_rpc_server::starknet::PendingBlockProvider::get_pending_transaction_by_index(&this, index)?
                {
                    return Ok(tx);
                }
            }

            let provider = this.storage().provider();
            let block_num = provider
                .convert_block_id(block_id)?
                .map(BlockHashOrNumber::Num)
                .ok_or(StarknetApiError::BlockNotFound)?;

            let tx = provider
                .transaction_by_block_and_idx(block_num, index)?
                .map(katana_rpc_types::transaction::RpcTxWithHash::from);

            tx.ok_or(StarknetApiError::InvalidTxnIndex)
        })
        .await?
    }

    async fn get_transaction_receipt(
        &self,
        transaction_hash: Felt,
    ) -> Result<katana_rpc_types::receipt::TxReceiptWithBlockInfo, StarknetApiError> {
        self.on_io_blocking_task(move |this| {
            if let Some(receipt) =
                katana_rpc_server::starknet::PendingBlockProvider::get_pending_receipt(
                    &this,
                    transaction_hash,
                )?
            {
                return Ok(receipt);
            }

            let provider = this.storage().provider();
            let receipt =
                katana_rpc_types_builder::ReceiptBuilder::new(transaction_hash, provider).build()?;

            receipt.ok_or(StarknetApiError::TxnHashNotFound)
        })
        .await?
    }

    async fn get_class(
        &self,
        block_id: BlockIdOrTag,
        class_hash: Felt,
    ) -> Result<katana_rpc_types::class::Class, StarknetApiError> {
        self.class_at_hash(block_id, class_hash).await
    }

    async fn get_class_hash_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Felt, StarknetApiError> {
        self.class_hash_at_address(block_id, contract_address).await
    }

    async fn get_class_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<katana_rpc_types::class::Class, StarknetApiError> {
        self.class_at_address(block_id, contract_address).await
    }

    async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> Result<u64, StarknetApiError> {
        self.block_tx_count(block_id).await
    }

    async fn call(
        &self,
        request: katana_rpc_types::FunctionCall,
        block_id: BlockIdOrTag,
    ) -> Result<katana_rpc_types::CallResponse, StarknetApiError> {
        // This requires access to the executor which is not directly available
        // For now, return unimplemented
        let _ = (request, block_id);
        Err(StarknetApiError::unexpected("call not yet implemented for gRPC"))
    }

    async fn estimate_fee(
        &self,
        _request: Vec<katana_rpc_types::broadcasted::BroadcastedTx>,
        _simulation_flags: Vec<katana_rpc_types::EstimateFeeSimulationFlag>,
        _block_id: BlockIdOrTag,
    ) -> Result<Vec<katana_rpc_types::FeeEstimate>, StarknetApiError> {
        Err(StarknetApiError::unexpected("estimate_fee not yet implemented for gRPC"))
    }

    async fn estimate_message_fee(
        &self,
        _message: katana_rpc_types::message::MsgFromL1,
        _block_id: BlockIdOrTag,
    ) -> Result<katana_rpc_types::FeeEstimate, StarknetApiError> {
        Err(StarknetApiError::unexpected("estimate_message_fee not yet implemented for gRPC"))
    }

    async fn get_events(
        &self,
        _filter: katana_rpc_types::event::EventFilterWithPage,
    ) -> Result<katana_rpc_types::event::GetEventsResponse, StarknetApiError> {
        Err(StarknetApiError::unexpected("get_events not yet implemented for gRPC"))
    }

    async fn get_nonce(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Felt, StarknetApiError> {
        self.nonce_at(block_id, contract_address).await
    }

    fn syncing(&self) -> bool {
        // Katana doesn't support syncing status yet
        false
    }
}
