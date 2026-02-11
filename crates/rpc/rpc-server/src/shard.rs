use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::ErrorObjectOwned;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::contract::{Nonce, StorageKey};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_api::shard::ShardApiServer;
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

/// Placeholder types — the actual `ShardRegistry` and `ShardScheduler` live in `katana-node`.
/// This module uses trait objects to avoid a circular dependency.
///
/// Trait for looking up per-shard StarknetApi instances.
pub trait ShardLookup: Send + Sync + 'static {
    /// Get the StarknetApi for a shard. Returns an RPC error if the shard doesn't exist.
    fn get_starknet_api(
        &self,
        shard_id: &ContractAddress,
    ) -> Result<Arc<dyn ShardStarknetApi>, ErrorObjectOwned>;

    /// Get or create the StarknetApi for a shard (for write operations).
    fn get_or_create_starknet_api(
        &self,
        shard_id: ContractAddress,
    ) -> Result<Arc<dyn ShardStarknetApi>, ErrorObjectOwned>;

    /// Schedule a shard for execution (after a write operation adds txs).
    fn schedule_shard(&self, shard_id: ContractAddress);

    /// List all shard ids.
    fn shard_ids(&self) -> Vec<ContractAddress>;

    /// Get the chain id.
    fn chain_id(&self) -> Felt;
}

/// Trait abstracting the per-shard StarknetApi methods we need to delegate to.
/// This avoids exposing concrete generic types from the node crate.
#[async_trait]
pub trait ShardStarknetApi: Send + Sync + 'static {
    async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> RpcResult<GetBlockWithTxHashesResponse>;

    async fn get_block_with_txs(&self, block_id: BlockIdOrTag)
        -> RpcResult<MaybePreConfirmedBlock>;

    async fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Felt>;

    async fn get_nonce(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<Nonce>;

    async fn get_transaction_by_hash(&self, tx_hash: TxHash) -> RpcResult<RpcTxWithHash>;

    async fn get_transaction_receipt(&self, tx_hash: TxHash) -> RpcResult<TxReceiptWithBlockInfo>;

    async fn get_transaction_status(&self, tx_hash: TxHash) -> RpcResult<TxStatus>;

    async fn call(&self, request: FunctionCall, block_id: BlockIdOrTag) -> RpcResult<CallResponse>;

    async fn get_events(&self, filter: EventFilterWithPage) -> RpcResult<GetEventsResponse>;

    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTx>,
        simulation_flags: Vec<EstimateFeeSimulationFlag>,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Vec<FeeEstimate>>;

    async fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumberResponse>;

    async fn block_number(&self) -> RpcResult<BlockNumberResponse>;

    async fn get_state_update(&self, block_id: BlockIdOrTag) -> RpcResult<StateUpdate>;

    async fn add_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse>;

    async fn add_declare_transaction(
        &self,
        tx: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse>;

    async fn add_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse>;

    async fn trace_transaction(&self, tx_hash: TxHash) -> RpcResult<TxTrace>;
}

/// The ShardRpcApi implementation that delegates to per-shard StarknetApi instances.
#[derive(Clone)]
pub struct ShardRpcApi<L: ShardLookup> {
    lookup: Arc<L>,
}

impl<L: ShardLookup> ShardRpcApi<L> {
    pub fn new(lookup: L) -> Self {
        Self { lookup: Arc::new(lookup) }
    }
}

#[async_trait]
impl<L: ShardLookup> ShardApiServer for ShardRpcApi<L> {
    async fn list_shards(&self) -> RpcResult<Vec<ContractAddress>> {
        Ok(self.lookup.shard_ids())
    }

    async fn get_block_with_tx_hashes(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<GetBlockWithTxHashesResponse> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_block_with_tx_hashes(block_id).await
    }

    async fn get_block_with_txs(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<MaybePreConfirmedBlock> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_block_with_txs(block_id).await
    }

    async fn get_storage_at(
        &self,
        shard_id: ContractAddress,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Felt> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_storage_at(contract_address, key, block_id).await
    }

    async fn get_nonce(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<Nonce> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_nonce(block_id, contract_address).await
    }

    async fn get_transaction_by_hash(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<RpcTxWithHash> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_transaction_by_hash(transaction_hash).await
    }

    async fn get_transaction_receipt(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxReceiptWithBlockInfo> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_transaction_receipt(transaction_hash).await
    }

    async fn get_transaction_status(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxStatus> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_transaction_status(transaction_hash).await
    }

    async fn call(
        &self,
        shard_id: ContractAddress,
        request: FunctionCall,
        block_id: BlockIdOrTag,
    ) -> RpcResult<CallResponse> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.call(request, block_id).await
    }

    async fn get_events(
        &self,
        shard_id: ContractAddress,
        filter: EventFilterWithPage,
    ) -> RpcResult<GetEventsResponse> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_events(filter).await
    }

    async fn estimate_fee(
        &self,
        shard_id: ContractAddress,
        request: Vec<BroadcastedTx>,
        simulation_flags: Vec<EstimateFeeSimulationFlag>,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Vec<FeeEstimate>> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.estimate_fee(request, simulation_flags, block_id).await
    }

    async fn block_hash_and_number(
        &self,
        shard_id: ContractAddress,
    ) -> RpcResult<BlockHashAndNumberResponse> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.block_hash_and_number().await
    }

    async fn block_number(&self, shard_id: ContractAddress) -> RpcResult<BlockNumberResponse> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.block_number().await
    }

    async fn chain_id(&self) -> RpcResult<Felt> {
        Ok(self.lookup.chain_id())
    }

    async fn get_state_update(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<StateUpdate> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.get_state_update(block_id).await
    }

    async fn add_invoke_transaction(
        &self,
        shard_id: ContractAddress,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        let api = self.lookup.get_or_create_starknet_api(shard_id)?;
        let result = api.add_invoke_transaction(invoke_transaction).await?;
        self.lookup.schedule_shard(shard_id);
        Ok(result)
    }

    async fn add_declare_transaction(
        &self,
        shard_id: ContractAddress,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse> {
        let api = self.lookup.get_or_create_starknet_api(shard_id)?;
        let result = api.add_declare_transaction(declare_transaction).await?;
        self.lookup.schedule_shard(shard_id);
        Ok(result)
    }

    async fn add_deploy_account_transaction(
        &self,
        shard_id: ContractAddress,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse> {
        let api = self.lookup.get_or_create_starknet_api(shard_id)?;
        let result = api.add_deploy_account_transaction(deploy_account_transaction).await?;
        self.lookup.schedule_shard(shard_id);
        Ok(result)
    }

    async fn trace_transaction(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxTrace> {
        let api = self.lookup.get_starknet_api(&shard_id)?;
        api.trace_transaction(transaction_hash).await
    }
}
