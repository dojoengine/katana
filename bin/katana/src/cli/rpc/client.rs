use anyhow::{anyhow, Context, Result};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_rpc_api::error::starknet::StarknetApiError;
use serde_json::Value;
use url::Url;

// Starknet RPC methods name
const RPC_SPEC_VERSION: &str = "starknet_specVersion";
const RPC_GET_BLOCK_WITH_TX_HASHES: &str = "starknet_getBlockWithTxHashes";
const RPC_GET_BLOCK_WITH_TXS: &str = "starknet_getBlockWithTxs";
const RPC_GET_BLOCK_WITH_RECEIPTS: &str = "starknet_getBlockWithReceipts";
const RPC_GET_STATE_UPDATE: &str = "starknet_getStateUpdate";
const RPC_GET_STORAGE_AT: &str = "starknet_getStorageAt";
const RPC_GET_TRANSACTION_BY_HASH: &str = "starknet_getTransactionByHash";
const RPC_GET_TRANSACTION_BY_BLOCK_ID_AND_INDEX: &str = "starknet_getTransactionByBlockIdAndIndex";
const RPC_GET_TRANSACTION_RECEIPT: &str = "starknet_getTransactionReceipt";
const RPC_GET_TRANSACTION_STATUS: &str = "starknet_getTransactionStatus";
const RPC_GET_CLASS: &str = "starknet_getClass";
const RPC_GET_CLASS_HASH_AT: &str = "starknet_getClassHashAt";
const RPC_GET_CLASS_AT: &str = "starknet_getClassAt";
const RPC_GET_BLOCK_TRANSACTION_COUNT: &str = "starknet_getBlockTransactionCount";
const RPC_CALL: &str = "starknet_call";
const RPC_BLOCK_NUMBER: &str = "starknet_blockNumber";
const RPC_BLOCK_HASH_AND_NUMBER: &str = "starknet_blockHashAndNumber";
const RPC_CHAIN_ID: &str = "starknet_chainId";
const RPC_SYNCING: &str = "starknet_syncing";
const RPC_GET_NONCE: &str = "starknet_getNonce";
const RPC_TRACE_TRANSACTION: &str = "starknet_traceTransaction";
const RPC_TRACE_BLOCK_TRANSACTIONS: &str = "starknet_traceBlockTransactions";

/// A JSON-RPC client for Starknet that returns raw JSON responses.
/// This is primarily used for debugging and validating RPC server responses.
#[derive(Debug, Clone)]
pub struct Client {
    client: HttpClient,
}

impl Client {
    pub fn new(url: Url) -> Result<Self> {
        HttpClientBuilder::default()
            .build(url)
            .map(|client| Self { client })
            .with_context("failed to build HTTP client")
    }
}

impl Client {
    async fn send_request<R, Params>(&self, method: &str, params: Params) -> Result<R>
    where
        R: serde::de::DeserializeOwned,
        Params: ToRpcParams + Send,
    {
        match self.client.request(method, params).await {
            Ok(res) => Ok(res),
            Err(err) => match err {
                jsonrpsee::core::client::Error::Call(error_object) => {
                    let error = StarknetApiError::from(error_object);
                    Err(anyhow!("Starknet error: {error}"))
                }
                other => Err(anyhow!("RPC error: {other}")),
            },
        }
    }

    // Read API methods

    pub async fn spec_version(&self) -> Result<Value> {
        self.send_request::<Value, _>(RPC_SPEC_VERSION, ArrayParams::new())
            .await
            .map_err(Into::into)
    }

    pub async fn get_block_with_tx_hashes(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(RPC_GET_BLOCK_WITH_TX_HASHES, params)
            .await
            .map_err(Into::into)
    }

    pub async fn get_block_with_txs(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(RPC_GET_BLOCK_WITH_TXS, params).await.map_err(Into::into)
    }

    pub async fn get_block_with_receipts(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(RPC_GET_BLOCK_WITH_RECEIPTS, params).await.map_err(Into::into)
    }

    pub async fn get_state_update(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.client.request::<Value, _>(RPC_GET_STATE_UPDATE, params).await.map_err(Into::into)
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
        self.client.request::<Value, _>(RPC_GET_STORAGE_AT, params).await.map_err(Into::into)
    }

    pub async fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", tx_hash))?;
        self.send_request::<Value, _>(RPC_GET_TRANSACTION_BY_HASH, params).await.map_err(Into::into)
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: Value,
        index: u64,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(index)?;
        self.send_request::<Value, _>(RPC_GET_TRANSACTION_BY_BLOCK_ID_AND_INDEX, params)
            .await
            .map_err(Into::into)
    }

    pub async fn get_transaction_receipt(&self, tx_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", tx_hash))?;
        self.send_request::<Value, _>(RPC_GET_TRANSACTION_RECEIPT, params).await.map_err(Into::into)
    }

    pub async fn get_transaction_status(&self, tx_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", tx_hash))?;
        self.send_request::<Value, _>(RPC_GET_TRANSACTION_STATUS, params).await.map_err(Into::into)
    }

    pub async fn get_class(&self, block_id: Value, class_hash: Felt) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(format!("{:#x}", class_hash))?;
        self.client.request::<Value, _>(RPC_GET_CLASS, params).await.map_err(Into::into)
    }

    pub async fn get_class_hash_at(
        &self,
        block_id: Value,
        contract_address: Felt,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(format!("{:#x}", contract_address))?;
        self.client.request::<Value, _>(RPC_GET_CLASS_HASH_AT, params).await.map_err(Into::into)
    }

    pub async fn get_class_at(&self, block_id: Value, contract_address: Felt) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(format!("{:#x}", contract_address))?;
        self.client.request::<Value, _>(RPC_GET_CLASS_AT, params).await.map_err(Into::into)
    }

    pub async fn get_block_transaction_count(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(RPC_GET_BLOCK_TRANSACTION_COUNT, params)
            .await
            .map_err(Into::into)
    }

    pub async fn call(&self, request: Value, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(request)?;
        params.insert(block_id)?;
        self.client.request::<Value, _>(RPC_CALL, params).await.map_err(Into::into)
    }

    pub async fn block_number(&self) -> Result<Value> {
        self.send_request::<Value, _>(RPC_BLOCK_NUMBER, ArrayParams::new())
            .await
            .map_err(Into::into)
    }

    pub async fn block_hash_and_number(&self) -> Result<Value> {
        self.send_request::<Value, _>(RPC_BLOCK_HASH_AND_NUMBER, ArrayParams::new())
            .await
            .map_err(Into::into)
    }

    pub async fn chain_id(&self) -> Result<Value> {
        self.send_request::<Value, _>(RPC_CHAIN_ID, ArrayParams::new()).await.map_err(Into::into)
    }

    pub async fn syncing(&self) -> Result<Value> {
        self.send_request::<Value, _>(RPC_SYNCING, ArrayParams::new()).await.map_err(Into::into)
    }

    pub async fn get_nonce(&self, block_id: Value, contract_address: Felt) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(format!("{:#x}", contract_address))?;
        self.client.request::<Value, _>(RPC_GET_NONCE, params).await.map_err(Into::into)
    }

    // Trace API methods

    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(format!("{:#x}", transaction_hash))?;
        self.send_request::<Value, _>(RPC_TRACE_TRANSACTION, params).await.map_err(Into::into)
    }

    pub async fn trace_block_transactions(&self, block_id: Value) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(RPC_TRACE_BLOCK_TRANSACTIONS, params)
            .await
            .map_err(Into::into)
    }
}
