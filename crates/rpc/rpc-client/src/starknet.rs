use jsonrpsee::http_client::HttpClient;
use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::{Nonce, StorageKey};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
pub use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet::{StarknetApiClient, StarknetTraceApiClient, StarknetWriteApiClient};
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
use katana_rpc_types::message::MsgFromL1;
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::state_update::StateUpdate;
use katana_rpc_types::trace::{
    SimulatedTransactionsResponse, TraceBlockTransactionsResponse, TxTrace,
};
use katana_rpc_types::transaction::RpcTxWithHash;
use katana_rpc_types::trie::{ContractStorageKeys, GetStorageProofResponse};
use katana_rpc_types::{
    CallResponse, EstimateFeeSimulationFlag, EventFilter, FeeEstimate, FunctionCall,
    ResultPageRequest, SimulationFlag, SyncingResponse, TxStatus,
};

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Client {
    client: HttpClient,
}

impl Client {
    pub fn new(client: HttpClient) -> Self {
        Client { client }
    }

    // Read API methods

    /// Returns the version of the Starknet JSON-RPC specification being used.
    pub async fn spec_version(&self) -> Result<String> {
        self.client.spec_version().await.map_err(Into::into)
    }

    /// Get block information with transaction hashes given the block id.
    pub async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<GetBlockWithTxHashesResponse> {
        self.client.get_block_with_tx_hashes(block_id).await.map_err(Into::into)
    }

    /// Get block information with full transactions given the block id.
    pub async fn get_block_with_txs(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePreConfirmedBlock> {
        self.client.get_block_with_txs(block_id).await.map_err(Into::into)
    }

    /// Get block information with full transactions and receipts given the block id.
    pub async fn get_block_with_receipts(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<GetBlockWithReceiptsResponse> {
        self.client.get_block_with_receipts(block_id).await.map_err(Into::into)
    }

    /// Get the information about the result of executing the requested block.
    pub async fn get_state_update(&self, block_id: BlockIdOrTag) -> Result<StateUpdate> {
        self.client.get_state_update(block_id).await.map_err(Into::into)
    }

    /// Get the value of the storage at the given address and key.
    pub async fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> Result<Felt> {
        self.client.get_storage_at(contract_address, key, block_id).await.map_err(Into::into)
    }

    /// Gets the transaction status (possibly reflecting that the tx is still in the mempool, or
    /// dropped from it).
    pub async fn get_transaction_status(&self, transaction_hash: TxHash) -> Result<TxStatus> {
        self.client.get_transaction_status(transaction_hash).await.map_err(Into::into)
    }

    /// Get the details and status of a submitted transaction.
    pub async fn get_transaction_by_hash(&self, transaction_hash: TxHash) -> Result<RpcTxWithHash> {
        self.client.get_transaction_by_hash(transaction_hash).await.map_err(Into::into)
    }

    /// Get the details of a transaction by a given block id and index.
    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        index: u64,
    ) -> Result<RpcTxWithHash> {
        self.client.get_transaction_by_block_id_and_index(block_id, index).await.map_err(Into::into)
    }

    /// Get the transaction receipt by the transaction hash.
    pub async fn get_transaction_receipt(
        &self,
        transaction_hash: TxHash,
    ) -> Result<TxReceiptWithBlockInfo> {
        self.client.get_transaction_receipt(transaction_hash).await.map_err(Into::into)
    }

    /// Get the contract class definition in the given block associated with the given hash.
    pub async fn get_class(&self, block_id: BlockIdOrTag, class_hash: ClassHash) -> Result<Class> {
        self.client.get_class(block_id, class_hash).await.map_err(Into::into)
    }

    /// Get the contract class hash in the given block for the contract deployed at the given
    /// address.
    pub async fn get_class_hash_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<ClassHash> {
        self.client.get_class_hash_at(block_id, contract_address).await.map_err(Into::into)
    }

    /// Get the contract class definition in the given block at the given address.
    pub async fn get_class_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Class> {
        self.client.get_class_at(block_id, contract_address).await.map_err(Into::into)
    }

    /// Get the compiled CASM code resulting from compiling a given class.
    pub async fn get_compiled_casm(&self, class_hash: ClassHash) -> Result<CasmClass> {
        self.client.get_compiled_casm(class_hash).await.map_err(Into::into)
    }

    /// Get the number of transactions in a block given a block id.
    pub async fn get_block_transaction_count(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<BlockTxCount> {
        self.client.get_block_transaction_count(block_id).await.map_err(Into::into)
    }

    /// Call a starknet function without creating a StarkNet transaction.
    pub async fn call(
        &self,
        request: FunctionCall,
        block_id: BlockIdOrTag,
    ) -> Result<CallResponse> {
        self.client.call(request, block_id).await.map_err(Into::into)
    }

    /// Estimate the fee for StarkNet transactions.
    pub async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTx>,
        simulation_flags: Vec<EstimateFeeSimulationFlag>,
        block_id: BlockIdOrTag,
    ) -> Result<Vec<FeeEstimate>> {
        self.client.estimate_fee(request, simulation_flags, block_id).await.map_err(Into::into)
    }

    /// Estimate the L2 fee of a message sent on L1.
    pub async fn estimate_message_fee(
        &self,
        message: MsgFromL1,
        block_id: BlockIdOrTag,
    ) -> Result<FeeEstimate> {
        self.client.estimate_message_fee(message, block_id).await.map_err(Into::into)
    }

    /// Get the most recent accepted block number.
    pub async fn block_number(&self) -> Result<BlockNumberResponse> {
        self.client.block_number().await.map_err(Into::into)
    }

    /// Get the most recent accepted block hash and number.
    pub async fn block_hash_and_number(&self) -> Result<BlockHashAndNumberResponse> {
        self.client.block_hash_and_number().await.map_err(Into::into)
    }

    /// Return the currently configured StarkNet chain id.
    pub async fn chain_id(&self) -> Result<Felt> {
        self.client.chain_id().await.map_err(Into::into)
    }

    /// Returns an object about the sync status, or false if the node is not syncing.
    pub async fn syncing(&self) -> Result<SyncingResponse> {
        self.client.syncing().await.map_err(Into::into)
    }

    /// Returns all event objects matching the conditions in the provided filter.
    pub async fn get_events(
        &self,
        event_filter: EventFilter,
        continuation_token: Option<String>,
        chunk_size: u64,
    ) -> Result<GetEventsResponse> {
        let page = ResultPageRequest { chunk_size, continuation_token };
        let request = EventFilterWithPage { event_filter, result_page_request: page };
        self.client.get_events(request).await.map_err(Into::into)
    }

    /// Get the nonce associated with the given address in the given block.
    pub async fn get_nonce(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Nonce> {
        self.client.get_nonce(block_id, contract_address).await.map_err(Into::into)
    }

    /// Get merkle paths in one of the state tries: global state, classes, individual contract.
    pub async fn get_storage_proof(
        &self,
        block_id: BlockIdOrTag,
        class_hashes: Option<Vec<ClassHash>>,
        contract_addresses: Option<Vec<ContractAddress>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeys>>,
    ) -> Result<GetStorageProofResponse> {
        self.client
            .get_storage_proof(block_id, class_hashes, contract_addresses, contracts_storage_keys)
            .await
            .map_err(Into::into)
    }

    // Write API methods

    /// Submit a new transaction to be added to the chain.
    pub async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> Result<AddInvokeTransactionResponse> {
        self.client.add_invoke_transaction(invoke_transaction).await.map_err(Into::into)
    }

    /// Submit a new class declaration transaction.
    pub async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> Result<AddDeclareTransactionResponse> {
        self.client.add_declare_transaction(declare_transaction).await.map_err(Into::into)
    }

    /// Submit a new deploy account transaction.
    pub async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> Result<AddDeployAccountTransactionResponse> {
        self.client
            .add_deploy_account_transaction(deploy_account_transaction)
            .await
            .map_err(Into::into)
    }

    // Trace API methods

    /// Returns the execution trace of the transaction designated by the input hash.
    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<TxTrace> {
        self.client.trace_transaction(transaction_hash).await.map_err(Into::into)
    }

    /// Simulates a list of transactions on the provided block.
    pub async fn simulate_transactions(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> Result<SimulatedTransactionsResponse> {
        self.client
            .simulate_transactions(block_id, transactions, simulation_flags)
            .await
            .map_err(Into::into)
    }

    /// Returns the execution traces of all transactions included in the given block.
    pub async fn trace_block_transactions(
        &self,
        block_id: ConfirmedBlockIdOrTag,
    ) -> Result<TraceBlockTransactionsResponse> {
        self.client.trace_block_transactions(block_id).await.map_err(Into::into)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// API-specific error returned by the server.
    #[error(transparent)]
    Starknet(StarknetApiError),

    /// Transport or other client-level error.
    #[error(transparent)]
    Client(jsonrpsee::core::client::Error),
}

impl From<jsonrpsee::core::client::Error> for Error {
    fn from(err: jsonrpsee::core::client::Error) -> Self {
        match err {
            jsonrpsee::core::client::Error::Call(err_obj) => Error::Starknet(err_obj.into()),
            _ => Error::Client(err),
        }
    }
}
