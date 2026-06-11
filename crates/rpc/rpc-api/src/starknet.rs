//! Starknet JSON-RPC specifications: <https://github.com/starkware-libs/starknet-specs>

use jsonrpsee::core::{RpcResult, SubscriptionResult};
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::{Nonce, StorageKey};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::block::{
    BlockHashAndNumberResponse, BlockNumberResponse, BlockTxCount, GetBlockWithReceiptsResponse,
    GetBlockWithTxHashesResponse, MaybePreConfirmedBlock,
};
use katana_rpc_types::broadcasted::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, BroadcastedDeclareTx, BroadcastedDeployAccountTx,
    BroadcastedInvokeTx, BroadcastedTx,
};
use katana_rpc_types::class::{CasmClass, Class};
use katana_rpc_types::event::{EventFilterWithPage, GetEventsResponse};
use katana_rpc_types::message::{MessageStatus, MsgFromL1};
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::state_update::StateUpdate;
// Used by the `#[subscription(item = ...)]` proc macro attribute.
#[allow(unused_imports)]
use katana_rpc_types::subscription::{
    EmittedEventWithFinalityStatus, SubscriptionBlockHeader, TransactionStatusUpdate,
    TxWithFinalityStatus,
};
use katana_rpc_types::trace::{
    SimulatedTransactionsResponse, TraceBlockTransactionsResponse, TxTrace,
};
use katana_rpc_types::transaction::RpcTxWithHash;
use katana_rpc_types::trie::{ContractStorageKeys, GetStorageProofResponse};
use katana_rpc_types::{
    CallResponse, EstimateFeeSimulationFlag, FeeEstimate, FunctionCall, SimulationFlag,
    SyncingResponse, TxStatus,
};

/// The currently supported version of the Starknet JSON-RPC specification.
pub const RPC_SPEC_VERSION: &str = "0.10.0";

/// Starknet JSON-RPC API.
///
/// Combines the read, write, and trace method groups defined by the upstream
/// Starknet JSON-RPC specification under a single trait. All methods share the
/// `starknet` namespace.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "starknet"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "starknet"))]
pub trait StarknetApi {
    ////////////////////////////////////////////////////////////////////////////
    // Read API methods
    ////////////////////////////////////////////////////////////////////////////

    /// Returns the version of the Starknet JSON-RPC specification being used.
    #[method(name = "specVersion")]
    async fn spec_version(&self) -> RpcResult<String> {
        Ok(RPC_SPEC_VERSION.into())
    }

    /// Get block information with transaction hashes given the block id.
    #[method(name = "getBlockWithTxHashes")]
    async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> RpcResult<GetBlockWithTxHashesResponse>;

    /// Get block information with full transactions given the block id.
    #[method(name = "getBlockWithTxs")]
    async fn get_block_with_txs(&self, block_id: BlockIdOrTag)
        -> RpcResult<MaybePreConfirmedBlock>;

    /// Get block information with full transactions and receipts given the block id.
    #[method(name = "getBlockWithReceipts")]
    async fn get_block_with_receipts(
        &self,
        block_id: BlockIdOrTag,
    ) -> RpcResult<GetBlockWithReceiptsResponse>;

    /// Get the information about the result of executing the requested block.
    #[method(name = "getStateUpdate")]
    async fn get_state_update(&self, block_id: BlockIdOrTag) -> RpcResult<StateUpdate>;

    /// Get the value of the storage at the given address and key
    #[method(name = "getStorageAt")]
    async fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Felt>;

    /// Gets the transaction status (possibly reflecting that the tx is still in the mempool, or
    /// dropped from it).
    #[method(name = "getTransactionStatus")]
    async fn get_transaction_status(&self, transaction_hash: TxHash) -> RpcResult<TxStatus>;

    /// Returns the status of every L2 L1Handler transaction spawned from the given
    /// settlement chain (L1) transaction. Returns an empty list if the L1 transaction
    /// is unknown to this node, either because it hasn't been ingested yet or because
    /// it never emitted any `MessageSent`/`LogMessageToL2` events.
    ///
    /// `transaction_hash` is the raw 32-byte L1 transaction hash. We use `B256` (not
    /// `Felt`) because Ethereum L1 hashes can exceed STARK_PRIME and modular reduction
    /// would corrupt the lookup key.
    #[method(name = "getMessagesStatus")]
    async fn get_messages_status(
        &self,
        transaction_hash: katana_primitives::B256,
    ) -> RpcResult<Vec<MessageStatus>>;

    /// Get the details and status of a submitted transaction.
    #[method(name = "getTransactionByHash")]
    async fn get_transaction_by_hash(&self, transaction_hash: TxHash) -> RpcResult<RpcTxWithHash>;

    /// Get the details of a transaction by a given block id and index.
    #[method(name = "getTransactionByBlockIdAndIndex")]
    async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        index: u64,
    ) -> RpcResult<RpcTxWithHash>;

    /// Get the transaction receipt by the transaction hash.
    #[method(name = "getTransactionReceipt")]
    async fn get_transaction_receipt(
        &self,
        transaction_hash: TxHash,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Get the contract class definition in the given block associated with the given hash.
    #[method(name = "getClass")]
    async fn get_class(&self, block_id: BlockIdOrTag, class_hash: ClassHash) -> RpcResult<Class>;

    /// Get the contract class hash in the given block for the contract deployed at the given
    /// address.
    #[method(name = "getClassHashAt")]
    async fn get_class_hash_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<ClassHash>;

    /// Get the contract class definition in the given block at the given address.
    #[method(name = "getClassAt")]
    async fn get_class_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<Class>;

    /// Get the compiled CASM code resulting from compiling a given class.
    #[method(name = "getCompiledCasm")]
    async fn get_compiled_casm(&self, class_hash: ClassHash) -> RpcResult<CasmClass>;

    /// Get the number of transactions in a block given a block id.
    #[method(name = "getBlockTransactionCount")]
    async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> RpcResult<BlockTxCount>;

    /// Call a starknet function without creating a StarkNet transaction.
    #[method(name = "call")]
    async fn call(&self, request: FunctionCall, block_id: BlockIdOrTag) -> RpcResult<CallResponse>;

    /// Estimate the fee for of StarkNet transactions.
    #[method(name = "estimateFee")]
    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTx>,
        simulation_flags: Vec<EstimateFeeSimulationFlag>,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Vec<FeeEstimate>>;

    /// Estimate the L2 fee of a message sent on L1.
    #[method(name = "estimateMessageFee")]
    async fn estimate_message_fee(
        &self,
        message: MsgFromL1,
        block_id: BlockIdOrTag,
    ) -> RpcResult<FeeEstimate>;

    /// Get the most recent accepted block number.
    #[method(name = "blockNumber")]
    async fn block_number(&self) -> RpcResult<BlockNumberResponse>;

    /// Get the most recent accepted block hash and number.
    #[method(name = "blockHashAndNumber")]
    async fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumberResponse>;

    /// Return the currently configured StarkNet chain id.
    #[method(name = "chainId")]
    async fn chain_id(&self) -> RpcResult<Felt>;

    /// Returns an object about the sync status, or false if the node is not synching.
    #[method(name = "syncing")]
    async fn syncing(&self) -> RpcResult<SyncingResponse> {
        Ok(SyncingResponse::NotSyncing)
    }

    /// Returns all event objects matching the conditions in the provided filter.
    #[method(name = "getEvents")]
    async fn get_events(&self, filter: EventFilterWithPage) -> RpcResult<GetEventsResponse>;

    /// Get the nonce associated with the given address in the given block.
    #[method(name = "getNonce")]
    async fn get_nonce(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> RpcResult<Nonce>;

    /// Get merkle paths in one of the state tries: global state, classes, individual contract. A
    /// single request can query for any mix of the three types of storage proofs (classes,
    /// contracts, and storage).
    #[method(name = "getStorageProof")]
    async fn get_storage_proof(
        &self,
        block_id: BlockIdOrTag,
        class_hashes: Option<Vec<ClassHash>>,
        contract_addresses: Option<Vec<ContractAddress>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeys>>,
    ) -> RpcResult<GetStorageProofResponse>;

    ////////////////////////////////////////////////////////////////////////////
    // Write API methods
    ////////////////////////////////////////////////////////////////////////////

    /// Submit a new transaction to be added to the chain.
    #[method(name = "addInvokeTransaction")]
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse>;

    /// Submit a new class declaration transaction.
    #[method(name = "addDeclareTransaction")]
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse>;

    /// Submit a new deploy account transaction.
    #[method(name = "addDeployAccountTransaction")]
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse>;

    ////////////////////////////////////////////////////////////////////////////
    // Trace API methods
    ////////////////////////////////////////////////////////////////////////////

    /// Returns the execution trace of the transaction designated by the input hash.
    #[method(name = "traceTransaction")]
    async fn trace_transaction(&self, transaction_hash: TxHash) -> RpcResult<TxTrace>;

    /// Simulates a list of transactions on the provided block.
    #[method(name = "simulateTransactions")]
    async fn simulate_transactions(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<SimulatedTransactionsResponse>;

    /// Returns the execution traces of all transactions included in the given block.
    #[method(name = "traceBlockTransactions")]
    async fn trace_block_transactions(
        &self,
        block_id: ConfirmedBlockIdOrTag,
    ) -> RpcResult<TraceBlockTransactionsResponse>;
}

/// WebSocket Subscription API.
///
/// Spec: <https://github.com/starkware-libs/starknet-specs/blob/v0.9.0/api/starknet_ws_api.json>
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "starknet"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "starknet"))]
pub trait StarknetSubscriptionApi {
    /// Subscribe to new block headers. Emits a [`SubscriptionBlockHeader`] for each new block.
    ///
    /// ## Parameters
    ///
    /// * `block_id` — Optional starting block for historical backfill. When provided, the server
    ///   replays headers from that block up to the current tip before switching to live streaming.
    #[subscription(
        name = "subscribeNewHeads" => "subscriptionNewHeads",
        unsubscribe = "unsubscribe",
        item = SubscriptionBlockHeader
    )]
    async fn subscribe_new_heads(&self, block_id: Option<BlockIdOrTag>) -> SubscriptionResult;

    /// Subscribe to emitted events. Emits an [`EmittedEventWithFinalityStatus`] for each matching
    /// event.
    ///
    /// ## Parameters
    ///
    /// * `from_address` — If set, only events emitted by this contract address are delivered.
    /// * `keys` — Positional key filter as defined by the Starknet spec. Each element is an array
    ///   of acceptable values for that key position; an empty inner array means "any value" at that
    ///   position. If the outer array is `None`, no key filtering is applied.
    /// * `block_id` — Optional starting block for historical backfill (same semantics and limits as
    ///   `subscribe_new_heads`).
    #[subscription(
        name = "subscribeEvents" => "subscriptionEvents",
        unsubscribe = "unsubscribeEvents",
        item = EmittedEventWithFinalityStatus
    )]
    async fn subscribe_events(
        &self,
        from_address: Option<ContractAddress>,
        keys: Option<Vec<Vec<Felt>>>,
        block_id: Option<BlockIdOrTag>,
    ) -> SubscriptionResult;

    /// Subscribe to status updates for a specific transaction. Emits a
    /// [`TransactionStatusUpdate`] each time the transaction's finality or execution status
    /// changes (e.g. `RECEIVED` → `ACCEPTED_ON_L2`).
    ///
    /// ## Parameters
    ///
    /// * `transaction_hash` — The hash of the transaction to track. The server immediately emits
    ///   the current status if the transaction is already known (in the mempool or in storage),
    ///   then continues to emit updates until the transaction reaches a final state.
    #[subscription(
        name = "subscribeTransactionStatus" => "subscriptionTransactionStatus",
        unsubscribe = "unsubscribeTransactionStatus",
        item = TransactionStatusUpdate
    )]
    async fn subscribe_transaction_status(&self, transaction_hash: TxHash) -> SubscriptionResult;

    /// Subscribe to new transaction receipts. Emits a [`TxReceiptWithBlockInfo`] for each
    /// transaction included in a new block.
    ///
    /// ## Parameters
    ///
    /// * `sender_address` — If set, only receipts for transactions sent by one of these addresses
    ///   are delivered. The spec defines a `TOO_MANY_ADDRESSES_IN_FILTER` (code 67) error when the
    ///   list is too large (server-enforced limit).
    #[subscription(
        name = "subscribeNewTransactionReceipts" => "subscriptionNewTransactionReceipts",
        unsubscribe = "unsubscribeNewTransactionReceipts",
        item = TxReceiptWithBlockInfo
    )]
    async fn subscribe_new_transaction_receipts(
        &self,
        sender_address: Option<Vec<ContractAddress>>,
    ) -> SubscriptionResult;

    /// Subscribe to new transactions. Emits a [`TxWithFinalityStatus`] for each transaction
    /// included in a new block.
    ///
    /// ## Parameters
    ///
    /// * `sender_address` — If set, only transactions sent by one of these addresses are delivered.
    ///   Same `TOO_MANY_ADDRESSES_IN_FILTER` (code 67) limit applies.
    #[subscription(
        name = "subscribeNewTransactions" => "subscriptionNewTransaction",
        unsubscribe = "unsubscribeNewTransactions",
        item = TxWithFinalityStatus
    )]
    async fn subscribe_new_transactions(
        &self,
        sender_address: Option<Vec<ContractAddress>>,
    ) -> SubscriptionResult;
}
