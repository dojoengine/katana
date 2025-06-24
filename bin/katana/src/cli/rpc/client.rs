use anyhow::Result;
use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use starknet::core::types::{
    BlockHashAndNumber, BlockId, ContractClass, FunctionCall, MaybePendingBlockWithReceipts,
    MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, MaybePendingStateUpdate,
    SyncStatusType, Transaction, TransactionReceiptWithBlockInfo, TransactionStatus,
    TransactionTrace, TransactionTraceWithHash,
};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, Url};

#[derive(Debug)]
pub struct Client {
    rpc_client: JsonRpcClient<HttpTransport>,
}

impl Client {
    pub fn new(url: Url) -> Self {
        Self { rpc_client: JsonRpcClient::new(HttpTransport::new(url)) }
    }

    // Read API methods

    pub async fn spec_version(&self) -> Result<String> {
        self.rpc_client
            .spec_version()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get spec version: {}", e))
    }

    pub async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockId,
    ) -> Result<MaybePendingBlockWithTxHashes> {
        self.rpc_client
            .get_block_with_tx_hashes(block_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get block with tx hashes: {}", e))
    }

    pub async fn get_block_with_txs(&self, block_id: BlockId) -> Result<MaybePendingBlockWithTxs> {
        self.rpc_client
            .get_block_with_txs(block_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get block with txs: {}", e))
    }

    pub async fn get_block_with_receipts(
        &self,
        block_id: BlockId,
    ) -> Result<MaybePendingBlockWithReceipts> {
        self.rpc_client
            .get_block_with_receipts(block_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get block with receipts: {}", e))
    }

    pub async fn get_state_update(&self, block_id: BlockId) -> Result<MaybePendingStateUpdate> {
        self.rpc_client
            .get_state_update(block_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get state update: {}", e))
    }

    pub async fn get_storage_at(
        &self,
        contract_address: Felt,
        key: Felt,
        block_id: BlockId,
    ) -> Result<Felt> {
        self.rpc_client
            .get_storage_at(contract_address, key, block_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get storage at: {}", e))
    }

    pub async fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Result<Transaction> {
        self.rpc_client
            .get_transaction_by_hash(tx_hash)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get transaction by hash: {}", e))
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockId,
        index: u64,
    ) -> Result<Transaction> {
        self.rpc_client
            .get_transaction_by_block_id_and_index(block_id, index)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get transaction by block id and index: {}", e))
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: TxHash,
    ) -> Result<TransactionReceiptWithBlockInfo> {
        self.rpc_client
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get transaction receipt: {}", e))
    }

    pub async fn get_transaction_status(&self, tx_hash: TxHash) -> Result<TransactionStatus> {
        self.rpc_client
            .get_transaction_status(tx_hash)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get transaction status: {}", e))
    }

    pub async fn get_class(&self, block_id: BlockId, class_hash: Felt) -> Result<ContractClass> {
        self.rpc_client
            .get_class(block_id, class_hash)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get class: {}", e))
    }

    pub async fn get_class_hash_at(
        &self,
        block_id: BlockId,
        contract_address: Felt,
    ) -> Result<Felt> {
        self.rpc_client
            .get_class_hash_at(block_id, contract_address)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get class hash at: {}", e))
    }

    pub async fn get_class_at(
        &self,
        block_id: BlockId,
        contract_address: Felt,
    ) -> Result<ContractClass> {
        self.rpc_client
            .get_class_at(block_id, contract_address)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get class at: {}", e))
    }

    pub async fn get_block_transaction_count(&self, block_id: BlockId) -> Result<u64> {
        self.rpc_client
            .get_block_transaction_count(block_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get block transaction count: {}", e))
    }

    pub async fn call(&self, request: FunctionCall, block_id: BlockId) -> Result<Vec<Felt>> {
        self.rpc_client
            .call(request, block_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to call: {}", e))
    }

    pub async fn block_number(&self) -> Result<BlockNumber> {
        self.rpc_client
            .block_number()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get block number: {}", e))
    }

    pub async fn block_hash_and_number(&self) -> Result<BlockHashAndNumber> {
        self.rpc_client
            .block_hash_and_number()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get block hash and number: {}", e))
    }

    pub async fn chain_id(&self) -> Result<Felt> {
        self.rpc_client
            .chain_id()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get chain id: {}", e))
    }

    pub async fn syncing(&self) -> Result<SyncStatusType> {
        self.rpc_client
            .syncing()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get syncing status: {}", e))
    }

    pub async fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> Result<Felt> {
        self.rpc_client
            .get_nonce(block_id, contract_address)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get nonce: {}", e))
    }

    // Trace API methods

    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<TransactionTrace> {
        self.rpc_client
            .trace_transaction(transaction_hash)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to trace transaction: {}", e))
    }

    pub async fn trace_block_transactions(
        &self,
        block_id: BlockId,
    ) -> Result<Vec<TransactionTraceWithHash>> {
        self.rpc_client
            .trace_block_transactions(block_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to trace block transactions: {}", e))
    }
}
