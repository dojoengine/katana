//! Starknet JSON-RPC API v0.10.0 trait definitions.

use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::{Nonce, StorageKey};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::broadcasted::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, BroadcastedDeclareTx, BroadcastedDeployAccountTx,
    BroadcastedInvokeTx, BroadcastedTx,
};
use katana_rpc_types::class::{CasmClass, Class};
use katana_rpc_types::message::MsgFromL1;
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::trace::{
    SimulatedTransactionsResponse, TraceBlockTransactionsResponse, TxTrace,
};
use katana_rpc_types::transaction::RpcTxWithHash;
use katana_rpc_types::trie::{ContractStorageKeys, GetStorageProofResponse};
// v0.10-specific types
use katana_rpc_types::v0_10::block::{
    BlockHashAndNumberResponse, BlockNumberResponse, BlockTxCount, GetBlockWithReceiptsResponse,
    GetBlockWithTxHashesResponse, MaybePreConfirmedBlock,
};
use katana_rpc_types::v0_10::event::{EventFilterWithPage, GetEventsResponse};
use katana_rpc_types::v0_10::state_update::StateUpdate;
use katana_rpc_types::{
    CallResponse, EstimateFeeSimulationFlag, FeeEstimate, FunctionCall, SimulationFlag,
    SyncingResponse, TxStatus,
};

pub const RPC_SPEC_VERSION: &str = "0.10.0";

/// Read API.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "starknet"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "starknet"))]
pub trait StarknetApi {
    #[method(name = "specVersion")]
    async fn spec_version(&self) -> RpcResult<String> {
        Ok(RPC_SPEC_VERSION.into())
    }

    #[method(name = "getBlockWithTxHashes")]
    async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> RpcResult<GetBlockWithTxHashesResponse>;

    #[method(name = "getBlockWithTxs")]
    async fn get_block_with_txs(&self, block_id: BlockIdOrTag)
        -> RpcResult<MaybePreConfirmedBlock>;

    #[method(name = "getBlockWithReceipts")]
    async fn get_block_with_receipts(
        &self,
        block_id: BlockIdOrTag,
    ) -> RpcResult<GetBlockWithReceiptsResponse>;

    #[method(name = "getStateUpdate")]
    async fn get_state_update(&self, block_id: BlockIdOrTag) -> RpcResult<StateUpdate>;

    #[method(name = "getStorageAt")]
    async fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Felt>;

    #[method(name = "getTransactionStatus")]
    async fn get_transaction_status(&self, transaction_hash: TxHash) -> RpcResult<TxStatus>;

    #[method(name = "getTransactionByHash")]
    async fn get_transaction_by_hash(&self, transaction_hash: TxHash) -> RpcResult<RpcTxWithHash>;

    #[method(name = "getTransactionByBlockIdAndIndex")]
    async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        index: u64,
    ) -> RpcResult<RpcTxWithHash>;

    #[method(name = "getTransactionReceipt")]
    async fn get_transaction_receipt(
        &self,
        transaction_hash: TxHash,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    #[method(name = "getClass")]
    async fn get_class(&self, block_id: BlockIdOrTag, class_hash: ClassHash) -> RpcResult<Class>;

    #[method(name = "getClassHashAt")]
    async fn get_class_hash_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<ClassHash>;

    #[method(name = "getClassAt")]
    async fn get_class_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<Class>;

    #[method(name = "getCompiledCasm")]
    async fn get_compiled_casm(&self, class_hash: ClassHash) -> RpcResult<CasmClass>;

    #[method(name = "getBlockTransactionCount")]
    async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> RpcResult<BlockTxCount>;

    #[method(name = "call")]
    async fn call(&self, request: FunctionCall, block_id: BlockIdOrTag) -> RpcResult<CallResponse>;

    #[method(name = "estimateFee")]
    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTx>,
        simulation_flags: Vec<EstimateFeeSimulationFlag>,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Vec<FeeEstimate>>;

    #[method(name = "estimateMessageFee")]
    async fn estimate_message_fee(
        &self,
        message: MsgFromL1,
        block_id: BlockIdOrTag,
    ) -> RpcResult<FeeEstimate>;

    #[method(name = "blockNumber")]
    async fn block_number(&self) -> RpcResult<BlockNumberResponse>;

    #[method(name = "blockHashAndNumber")]
    async fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumberResponse>;

    #[method(name = "chainId")]
    async fn chain_id(&self) -> RpcResult<Felt>;

    #[method(name = "syncing")]
    async fn syncing(&self) -> RpcResult<SyncingResponse> {
        Ok(SyncingResponse::NotSyncing)
    }

    #[method(name = "getEvents")]
    async fn get_events(&self, filter: EventFilterWithPage) -> RpcResult<GetEventsResponse>;

    #[method(name = "getNonce")]
    async fn get_nonce(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<Nonce>;

    #[method(name = "getStorageProof")]
    async fn get_storage_proof(
        &self,
        block_id: BlockIdOrTag,
        class_hashes: Option<Vec<ClassHash>>,
        contract_addresses: Option<Vec<ContractAddress>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeys>>,
    ) -> RpcResult<GetStorageProofResponse>;
}

/// Write API.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "starknet"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "starknet"))]
pub trait StarknetWriteApi {
    #[method(name = "addInvokeTransaction")]
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse>;

    #[method(name = "addDeclareTransaction")]
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse>;

    #[method(name = "addDeployAccountTransaction")]
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse>;
}

/// Trace API.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "starknet"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "starknet"))]
pub trait StarknetTraceApi {
    #[method(name = "traceTransaction")]
    async fn trace_transaction(&self, transaction_hash: TxHash) -> RpcResult<TxTrace>;

    #[method(name = "simulateTransactions")]
    async fn simulate_transactions(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<SimulatedTransactionsResponse>;

    #[method(name = "traceBlockTransactions")]
    async fn trace_block_transactions(
        &self,
        block_id: ConfirmedBlockIdOrTag,
    ) -> RpcResult<TraceBlockTransactionsResponse>;
}
