use anyhow::{Context, Result};
use katana_primitives::block::{BlockIdOrTag, BlockNumber};
use katana_primitives::class::ClassHash;
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::block::{
    BlockHashAndNumber, BlockTxCount, MaybePendingBlockWithReceipts, MaybePendingBlockWithTxHashes,
    MaybePendingBlockWithTxs,
};
use katana_rpc_types::class::RpcContractClass;
use katana_rpc_types::event::{EventFilterWithPage, EventsPage};
use katana_rpc_types::message::MsgFromL1;
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::state_update::MaybePendingStateUpdate;
use katana_rpc_types::transaction::{
    BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx, BroadcastedTx,
    DeclareTxResult, DeployAccountTxResult, InvokeTxResult, Tx,
};
use katana_rpc_types::trie::{ContractStorageKeys, GetStorageProofResponse};
use katana_rpc_types::{
    FeeEstimate, FeltAsHex, FunctionCall, SimulationFlag, SimulationFlagForEstimateFee,
    SyncingStatus,
};
use starknet::core::types::{
    SimulatedTransaction, TransactionStatus, TransactionTrace, TransactionTraceWithHash,
};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, Url};

pub struct RpcClient {
    client: JsonRpcClient<HttpTransport>,
}

impl RpcClient {
    pub fn new(url: &str) -> Result<Self> {
        let url = Url::parse(url).context("Invalid URL format")?;
        let client = JsonRpcClient::new(HttpTransport::new(url));
        
        Ok(Self { client })
    }
    
    // Read API methods

    pub async fn spec_version(&self) -> Result<String> {
        self.client.spec_version().await.map_err(|e| anyhow::anyhow!("Failed to get spec version: {}", e))
    }

    pub async fn get_block_with_tx_hashes(&self, block_id: BlockIdOrTag) -> Result<MaybePendingBlockWithTxHashes> {
        self.client.get_block_with_tx_hashes(block_id).await.map_err(|e| anyhow::anyhow!("Failed to get block with tx hashes: {}", e))
    }
    
    pub async fn get_block_with_txs(&self, block_id: BlockIdOrTag) -> Result<MaybePendingBlockWithTxs> {
        self.client.get_block_with_txs(block_id).await.map_err(|e| anyhow::anyhow!("Failed to get block with txs: {}", e))
    }
    
    pub async fn get_block_with_receipts(&self, block_id: BlockIdOrTag) -> Result<MaybePendingBlockWithReceipts> {
        self.client.get_block_with_receipts(block_id).await.map_err(|e| anyhow::anyhow!("Failed to get block with receipts: {}", e))
    }

    pub async fn get_state_update(&self, block_id: BlockIdOrTag) -> Result<MaybePendingStateUpdate> {
        self.client.get_state_update(block_id).await.map_err(|e| anyhow::anyhow!("Failed to get state update: {}", e))
    }

    pub async fn get_storage_at(&self, contract_address: Felt, key: Felt, block_id: BlockIdOrTag) -> Result<FeltAsHex> {
        self.client.get_storage_at(contract_address, key, block_id).await.map_err(|e| anyhow::anyhow!("Failed to get storage at: {}", e))
    }
    
    pub async fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Result<Tx> {
        self.client.get_transaction_by_hash(tx_hash).await.map_err(|e| anyhow::anyhow!("Failed to get transaction by hash: {}", e))
    }

    pub async fn get_transaction_by_block_id_and_index(&self, block_id: BlockIdOrTag, index: u64) -> Result<Tx> {
        self.client.get_transaction_by_block_id_and_index(block_id, index).await.map_err(|e| anyhow::anyhow!("Failed to get transaction by block id and index: {}", e))
    }
    
    pub async fn get_transaction_receipt(&self, tx_hash: TxHash) -> Result<TxReceiptWithBlockInfo> {
        self.client.get_transaction_receipt(tx_hash).await.map_err(|e| anyhow::anyhow!("Failed to get transaction receipt: {}", e))
    }
    
    pub async fn get_transaction_status(&self, tx_hash: TxHash) -> Result<TransactionStatus> {
        self.client.get_transaction_status(tx_hash).await.map_err(|e| anyhow::anyhow!("Failed to get transaction status: {}", e))
    }

    pub async fn get_class(&self, block_id: BlockIdOrTag, class_hash: Felt) -> Result<RpcContractClass> {
        self.client.get_class(block_id, class_hash).await.map_err(|e| anyhow::anyhow!("Failed to get class: {}", e))
    }

    pub async fn get_class_hash_at(&self, block_id: BlockIdOrTag, contract_address: Felt) -> Result<FeltAsHex> {
        self.client.get_class_hash_at(block_id, contract_address).await.map_err(|e| anyhow::anyhow!("Failed to get class hash at: {}", e))
    }

    pub async fn get_class_at(&self, block_id: BlockIdOrTag, contract_address: Felt) -> Result<RpcContractClass> {
        self.client.get_class_at(block_id, contract_address).await.map_err(|e| anyhow::anyhow!("Failed to get class at: {}", e))
    }

    pub async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> Result<BlockTxCount> {
        self.client.get_block_transaction_count(block_id).await.map_err(|e| anyhow::anyhow!("Failed to get block transaction count: {}", e))
    }

    pub async fn call(&self, request: FunctionCall, block_id: BlockIdOrTag) -> Result<Vec<FeltAsHex>> {
        self.client.call(request, block_id).await.map_err(|e| anyhow::anyhow!("Failed to call: {}", e))
    }
    
    pub async fn estimate_fee(&self, request: Vec<BroadcastedTx>, simulation_flags: Vec<SimulationFlagForEstimateFee>, block_id: BlockIdOrTag) -> Result<Vec<FeeEstimate>> {
        self.client.estimate_fee(request, simulation_flags, block_id).await.map_err(|e| anyhow::anyhow!("Failed to estimate fee: {}", e))
    }

    pub async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockIdOrTag) -> Result<FeeEstimate> {
        self.client.estimate_message_fee(message, block_id).await.map_err(|e| anyhow::anyhow!("Failed to estimate message fee: {}", e))
    }

    pub async fn block_number(&self) -> Result<BlockNumber> {
        self.client.block_number().await.map_err(|e| anyhow::anyhow!("Failed to get block number: {}", e))
    }

    pub async fn block_hash_and_number(&self) -> Result<BlockHashAndNumber> {
        self.client.block_hash_and_number().await.map_err(|e| anyhow::anyhow!("Failed to get block hash and number: {}", e))
    }

    pub async fn chain_id(&self) -> Result<FeltAsHex> {
        self.client.chain_id().await.map_err(|e| anyhow::anyhow!("Failed to get chain id: {}", e))
    }

    pub async fn syncing(&self) -> Result<SyncingStatus> {
        self.client.syncing().await.map_err(|e| anyhow::anyhow!("Failed to get syncing status: {}", e))
    }

    pub async fn get_events(&self, filter: EventFilterWithPage) -> Result<EventsPage> {
        self.client.get_events(filter).await.map_err(|e| anyhow::anyhow!("Failed to get events: {}", e))
    }

    pub async fn get_nonce(&self, block_id: BlockIdOrTag, contract_address: Felt) -> Result<FeltAsHex> {
        self.client.get_nonce(block_id, contract_address).await.map_err(|e| anyhow::anyhow!("Failed to get nonce: {}", e))
    }

    pub async fn get_storage_proof(&self, block_id: BlockIdOrTag, class_hashes: Option<Vec<ClassHash>>, contract_addresses: Option<Vec<ContractAddress>>, contracts_storage_keys: Option<Vec<ContractStorageKeys>>) -> Result<GetStorageProofResponse> {
        self.client.get_storage_proof(block_id, class_hashes, contract_addresses, contracts_storage_keys).await.map_err(|e| anyhow::anyhow!("Failed to get storage proof: {}", e))
    }

    // Write API methods

    pub async fn add_invoke_transaction(&self, invoke_transaction: BroadcastedInvokeTx) -> Result<InvokeTxResult> {
        self.client.add_invoke_transaction(invoke_transaction).await.map_err(|e| anyhow::anyhow!("Failed to add invoke transaction: {}", e))
    }

    pub async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTx) -> Result<DeclareTxResult> {
        self.client.add_declare_transaction(declare_transaction).await.map_err(|e| anyhow::anyhow!("Failed to add declare transaction: {}", e))
    }

    pub async fn add_deploy_account_transaction(&self, deploy_account_transaction: BroadcastedDeployAccountTx) -> Result<DeployAccountTxResult> {
        self.client.add_deploy_account_transaction(deploy_account_transaction).await.map_err(|e| anyhow::anyhow!("Failed to add deploy account transaction: {}", e))
    }

    // Trace API methods

    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<TransactionTrace> {
        self.client.trace_transaction(transaction_hash).await.map_err(|e| anyhow::anyhow!("Failed to trace transaction: {}", e))
    }

    pub async fn simulate_transactions(&self, block_id: BlockIdOrTag, transactions: Vec<BroadcastedTx>, simulation_flags: Vec<SimulationFlag>) -> Result<Vec<SimulatedTransaction>> {
        self.client.simulate_transactions(block_id, transactions, simulation_flags).await.map_err(|e| anyhow::anyhow!("Failed to simulate transactions: {}", e))
    }

    pub async fn trace_block_transactions(&self, block_id: BlockIdOrTag) -> Result<Vec<TransactionTraceWithHash>> {
        self.client.trace_block_transactions(block_id).await.map_err(|e| anyhow::anyhow!("Failed to trace block transactions: {}", e))
    }
}