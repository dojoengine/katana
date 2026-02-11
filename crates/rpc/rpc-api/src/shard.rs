use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::contract::{Nonce, StorageKey};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
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

/// Shard API — routes requests to per-contract shard instances.
#[rpc(server, namespace = "shard")]
pub trait ShardApi {
    // -- Management --

    /// List all currently active shard ids.
    #[method(name = "listShards")]
    async fn list_shards(&self) -> RpcResult<Vec<ContractAddress>>;

    // -- Read (mirror starknet, prepend shard_id) --

    /// Get block information with transaction hashes for a specific shard.
    #[method(name = "getBlockWithTxHashes")]
    async fn get_block_with_tx_hashes(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<GetBlockWithTxHashesResponse>;

    /// Get block information with full transactions for a specific shard.
    #[method(name = "getBlockWithTxs")]
    async fn get_block_with_txs(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<MaybePreConfirmedBlock>;

    /// Get the value of the storage at the given address and key for a specific shard.
    #[method(name = "getStorageAt")]
    async fn get_storage_at(
        &self,
        shard_id: ContractAddress,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Felt>;

    /// Get the nonce associated with the given address for a specific shard.
    #[method(name = "getNonce")]
    async fn get_nonce(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<Nonce>;

    /// Get the details and status of a submitted transaction for a specific shard.
    #[method(name = "getTransactionByHash")]
    async fn get_transaction_by_hash(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<RpcTxWithHash>;

    /// Get the transaction receipt for a specific shard.
    #[method(name = "getTransactionReceipt")]
    async fn get_transaction_receipt(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Get the transaction status for a specific shard.
    #[method(name = "getTransactionStatus")]
    async fn get_transaction_status(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxStatus>;

    /// Call a starknet function without creating a transaction, on a specific shard.
    #[method(name = "call")]
    async fn call(
        &self,
        shard_id: ContractAddress,
        request: FunctionCall,
        block_id: BlockIdOrTag,
    ) -> RpcResult<CallResponse>;

    /// Returns all event objects matching the filter for a specific shard.
    #[method(name = "getEvents")]
    async fn get_events(
        &self,
        shard_id: ContractAddress,
        filter: EventFilterWithPage,
    ) -> RpcResult<GetEventsResponse>;

    /// Estimate the fee for StarkNet transactions on a specific shard.
    #[method(name = "estimateFee")]
    async fn estimate_fee(
        &self,
        shard_id: ContractAddress,
        request: Vec<BroadcastedTx>,
        simulation_flags: Vec<EstimateFeeSimulationFlag>,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Vec<FeeEstimate>>;

    /// Get the most recent accepted block hash and number for a specific shard.
    #[method(name = "blockHashAndNumber")]
    async fn block_hash_and_number(
        &self,
        shard_id: ContractAddress,
    ) -> RpcResult<BlockHashAndNumberResponse>;

    /// Get the most recent accepted block number for a specific shard.
    #[method(name = "blockNumber")]
    async fn block_number(&self, shard_id: ContractAddress) -> RpcResult<BlockNumberResponse>;

    /// Return the currently configured chain id.
    #[method(name = "chainId")]
    async fn chain_id(&self) -> RpcResult<Felt>;

    /// Get the state update for a specific shard.
    #[method(name = "getStateUpdate")]
    async fn get_state_update(
        &self,
        shard_id: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> RpcResult<StateUpdate>;

    // -- Write --

    /// Submit a new invoke transaction to a specific shard.
    #[method(name = "addInvokeTransaction")]
    async fn add_invoke_transaction(
        &self,
        shard_id: ContractAddress,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse>;

    /// Submit a new class declaration transaction to a specific shard.
    #[method(name = "addDeclareTransaction")]
    async fn add_declare_transaction(
        &self,
        shard_id: ContractAddress,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse>;

    /// Submit a new deploy account transaction to a specific shard.
    #[method(name = "addDeployAccountTransaction")]
    async fn add_deploy_account_transaction(
        &self,
        shard_id: ContractAddress,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse>;

    // -- Trace --

    /// Returns the execution trace of the transaction for a specific shard.
    #[method(name = "traceTransaction")]
    async fn trace_transaction(
        &self,
        shard_id: ContractAddress,
        transaction_hash: TxHash,
    ) -> RpcResult<TxTrace>;
}
