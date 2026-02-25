use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::ErrorObjectOwned;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::contract::{Nonce, StorageKey};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_api::shard::ShardApiServer;
use katana_rpc_api::starknet::{StarknetApiServer, StarknetTraceApiServer, StarknetWriteApiServer};
use katana_rpc_types::block::{
    BlockHashAndNumberResponse, BlockNumberResponse, GetBlockWithTxHashesResponse,
    MaybePreConfirmedBlock,
};
use katana_rpc_types::broadcasted::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, BroadcastedDeclareTx, BroadcastedDeployAccountTx,
    BroadcastedInvokeTx, BroadcastedTx,
};
use katana_rpc_types::event::{EventFilterWithPage, GetEventsResponse};
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::state_update::StateUpdate;
use katana_rpc_types::trace::TxTrace;
use katana_rpc_types::transaction::RpcTxWithHash;
use katana_rpc_types::{
    CallResponse, EstimateFeeSimulationFlag, FeeEstimate, FunctionCall, TxStatus,
};

/// Provides per-shard RPC capabilities to the [`ShardRpc`] implementation.
///
/// The node crate implements this trait to bridge its concrete shard types
/// (manager, scheduler) into the RPC layer without creating a circular dependency.
pub trait ShardProvider: Send + Sync + 'static {
    /// The per-shard API type that handles Starknet RPC calls.
    type Api: StarknetApiServer + StarknetWriteApiServer + StarknetTraceApiServer + Send + Sync;

    /// Resolve a shard's API by ID (for read operations).
    fn starknet_api(&self, shard_id: ContractAddress) -> Result<Self::Api, ErrorObjectOwned>;

    /// List all registered shard IDs.
    fn shard_ids(&self) -> Vec<ContractAddress>;

    /// Get the chain ID.
    fn chain_id(&self) -> Felt;
}

/// Implements `ShardApiServer` by delegating to a [`ShardProvider`].
pub struct ShardRpc<P> {
    provider: P,
}

impl<P: ShardProvider> ShardRpc<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl<P: ShardProvider> ShardApiServer for ShardRpc<P> {
    async fn list_shards(&self) -> RpcResult<Vec<ContractAddress>> {
        Ok(self.provider.shard_ids())
    }

    async fn get_block_with_tx_hashes(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<GetBlockWithTxHashesResponse> {
        StarknetApiServer::get_block_with_tx_hashes(
            &self.provider.starknet_api(shard_id)?,
            block_id,
        )
        .await
    }

    async fn get_block_with_txs(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<MaybePreConfirmedBlock> {
        StarknetApiServer::get_block_with_txs(&self.provider.starknet_api(shard_id)?, block_id)
            .await
    }

    async fn get_storage_at(
        &self,
        shard_id: ContractAddress,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Felt> {
        StarknetApiServer::get_storage_at(
            &self.provider.starknet_api(shard_id)?,
            contract_address,
            key,
            block_id,
        )
        .await
    }

    async fn get_nonce(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<Nonce> {
        StarknetApiServer::get_nonce(
            &self.provider.starknet_api(shard_id)?,
            block_id,
            contract_address,
        )
        .await
    }

    async fn get_transaction_by_hash(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<RpcTxWithHash> {
        StarknetApiServer::get_transaction_by_hash(
            &self.provider.starknet_api(shard_id)?,
            transaction_hash,
        )
        .await
    }

    async fn get_transaction_receipt(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxReceiptWithBlockInfo> {
        StarknetApiServer::get_transaction_receipt(
            &self.provider.starknet_api(shard_id)?,
            transaction_hash,
        )
        .await
    }

    async fn get_transaction_status(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxStatus> {
        StarknetApiServer::get_transaction_status(
            &self.provider.starknet_api(shard_id)?,
            transaction_hash,
        )
        .await
    }

    async fn call(
        &self,
        shard_id: ContractAddress,
        request: FunctionCall,
        block_id: BlockIdOrTag,
    ) -> RpcResult<CallResponse> {
        StarknetApiServer::call(&self.provider.starknet_api(shard_id)?, request, block_id).await
    }

    async fn get_events(
        &self,
        shard_id: ContractAddress,
        filter: EventFilterWithPage,
    ) -> RpcResult<GetEventsResponse> {
        StarknetApiServer::get_events(&self.provider.starknet_api(shard_id)?, filter).await
    }

    async fn estimate_fee(
        &self,
        shard_id: ContractAddress,
        request: Vec<BroadcastedTx>,
        simulation_flags: Vec<EstimateFeeSimulationFlag>,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Vec<FeeEstimate>> {
        StarknetApiServer::estimate_fee(
            &self.provider.starknet_api(shard_id)?,
            request,
            simulation_flags,
            block_id,
        )
        .await
    }

    async fn block_hash_and_number(
        &self,
        shard_id: ContractAddress,
    ) -> RpcResult<BlockHashAndNumberResponse> {
        StarknetApiServer::block_hash_and_number(&self.provider.starknet_api(shard_id)?).await
    }

    async fn block_number(&self, shard_id: ContractAddress) -> RpcResult<BlockNumberResponse> {
        StarknetApiServer::block_number(&self.provider.starknet_api(shard_id)?).await
    }

    async fn chain_id(&self) -> RpcResult<Felt> {
        Ok(self.provider.chain_id())
    }

    async fn get_state_update(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<StateUpdate> {
        StarknetApiServer::get_state_update(&self.provider.starknet_api(shard_id)?, block_id).await
    }

    async fn add_invoke_transaction(
        &self,
        shard_id: ContractAddress,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        let api = self.provider.starknet_api(shard_id)?;
        StarknetWriteApiServer::add_invoke_transaction(&api, invoke_transaction).await
    }

    async fn add_declare_transaction(
        &self,
        shard_id: ContractAddress,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse> {
        let api = self.provider.starknet_api(shard_id)?;
        StarknetWriteApiServer::add_declare_transaction(&api, declare_transaction).await
    }

    async fn add_deploy_account_transaction(
        &self,
        shard_id: ContractAddress,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse> {
        let api = self.provider.starknet_api(shard_id)?;
        StarknetWriteApiServer::add_deploy_account_transaction(&api, deploy_account_transaction)
            .await
    }

    async fn trace_transaction(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxTrace> {
        StarknetTraceApiServer::trace_transaction(
            &self.provider.starknet_api(shard_id)?,
            transaction_hash,
        )
        .await
    }
}
