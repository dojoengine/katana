use anyhow::{anyhow, Result};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use serde_json::Value;
use url::Url;

/// A JSON-RPC client for Starknet that returns raw JSON responses.
/// This is primarily used for debugging and validating RPC server responses.
#[derive(Debug, Clone)]
pub struct Client {
    client: HttpClient,
}

impl Client {
    pub fn new(url: Url) -> Result<Self> {
        let client = HttpClientBuilder::default()
            .build(url)
            .map_err(|e| anyhow!("Failed to build HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Client Starknet JSON-RPC implementations
////////////////////////////////////////////////////////////////////////////////////////////////////

impl Client {
    // Read API methods

    pub async fn spec_version(&self) -> Result<Value> {
        self.client
            .request::<Value>("starknet_specVersion", ArrayParams::new())
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_block_with_tx_hashes(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.client
            .request::<Value>("starknet_getBlockWithTxHashes", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_block_with_txs(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.client
            .request::<Value>("starknet_getBlockWithTxs", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_block_with_receipts(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.client
            .request::<Value>("starknet_getBlockWithReceipts", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_state_update(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.client
            .request::<Value>("starknet_getStateUpdate", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_storage_at(
        &self,
        contract_address: Felt,
        key: Felt,
        block_id: Value,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", contract_address))?;
        params.insert(format!("{:#x}", key))?;
        params.insert(block_id)?;
        self.client
            .request::<Value>("starknet_getStorageAt", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", tx_hash))?;
        self.client
            .request::<Value>("starknet_getTransactionByHash", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: Value,
        index: u64,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(index)?;
        self.client
            .request::<Value>("starknet_getTransactionByBlockIdAndIndex", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_transaction_receipt(&self, tx_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", tx_hash))?;
        self.client
            .request::<Value>("starknet_getTransactionReceipt", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_transaction_status(&self, tx_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", tx_hash))?;
        self.client
            .request::<Value>("starknet_getTransactionStatus", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_class(&self, block_id: Value, class_hash: Felt) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(format!("{:#x}", class_hash))?;
        self.client
            .request::<Value>("starknet_getClass", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_class_hash_at(
        &self,
        block_id: Value,
        contract_address: Felt,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(format!("{:#x}", contract_address))?;
        self.client
            .request::<Value>("starknet_getClassHashAt", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_class_at(&self, block_id: Value, contract_address: Felt) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(format!("{:#x}", contract_address))?;
        self.client
            .request::<Value>("starknet_getClassAt", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_block_transaction_count(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.client
            .request::<Value>("starknet_getBlockTransactionCount", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn call(&self, request: Value, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(request)?;
        params.insert(block_id)?;
        self.client
            .request::<Value>("starknet_call", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn block_number(&self) -> Result<Value> {
        self.client
            .request::<Value>("starknet_blockNumber", ArrayParams::new())
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn block_hash_and_number(&self) -> Result<Value> {
        self.client
            .request::<Value>("starknet_blockHashAndNumber", ArrayParams::new())
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn chain_id(&self) -> Result<Value> {
        self.client
            .request::<Value>("starknet_chainId", ArrayParams::new())
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn syncing(&self) -> Result<Value> {
        self.client
            .request::<Value>("starknet_syncing", ArrayParams::new())
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn get_nonce(&self, block_id: Value, contract_address: Felt) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(format!("{:#x}", contract_address))?;
        self.client
            .request::<Value>("starknet_getNonce", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    // Trace API methods

    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", transaction_hash))?;
        self.client
            .request::<Value>("starknet_traceTransaction", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }

    pub async fn trace_block_transactions(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.client
            .request::<Value>("starknet_traceBlockTransactions", params)
            .await
            .map_err(|e| anyhow!("RPC error: {e}"))
    }
}
