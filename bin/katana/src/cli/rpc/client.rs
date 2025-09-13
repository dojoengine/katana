use anyhow::{anyhow, Result};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::{ContractAddress, StorageKey};
use katana_primitives::transaction::TxHash;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::FunctionCall;
use serde_json::Value;
use url::Url;

// RPC method name constants
const SPEC_VERSION: &str = "starknet_specVersion";
const GET_BLOCK_WITH_TX_HASHES: &str = "starknet_getBlockWithTxHashes";
const GET_BLOCK_WITH_TXS: &str = "starknet_getBlockWithTxs";
const GET_BLOCK_WITH_RECEIPTS: &str = "starknet_getBlockWithReceipts";
const GET_STATE_UPDATE: &str = "starknet_getStateUpdate";
const GET_STORAGE_AT: &str = "starknet_getStorageAt";
const GET_TRANSACTION_STATUS: &str = "starknet_getTransactionStatus";
const GET_TRANSACTION_BY_HASH: &str = "starknet_getTransactionByHash";
const GET_TRANSACTION_BY_BLOCK_ID_AND_INDEX: &str = "starknet_getTransactionByBlockIdAndIndex";
const GET_TRANSACTION_RECEIPT: &str = "starknet_getTransactionReceipt";
const GET_CLASS: &str = "starknet_getClass";
const GET_CLASS_HASH_AT: &str = "starknet_getClassHashAt";
const GET_CLASS_AT: &str = "starknet_getClassAt";
const GET_BLOCK_TRANSACTION_COUNT: &str = "starknet_getBlockTransactionCount";
const CALL: &str = "starknet_call";
const BLOCK_NUMBER: &str = "starknet_blockNumber";
const BLOCK_HASH_AND_NUMBER: &str = "starknet_blockHashAndNumber";
const CHAIN_ID: &str = "starknet_chainId";
const SYNCING: &str = "starknet_syncing";
const GET_NONCE: &str = "starknet_getNonce";
const TRACE_TRANSACTION: &str = "starknet_traceTransaction";
const TRACE_BLOCK_TRANSACTIONS: &str = "starknet_traceBlockTransactions";

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
            .map_err(|e| anyhow!("failed to build HTTP client: {e}"))
    }

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
        self.send_request::<Value, _>(SPEC_VERSION, ArrayParams::new())
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_block_with_tx_hashes(&self, block_id: BlockIdOrTag) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(GET_BLOCK_WITH_TX_HASHES, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_block_with_txs(&self, block_id: BlockIdOrTag) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(GET_BLOCK_WITH_TXS, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_block_with_receipts(&self, block_id: BlockIdOrTag) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(GET_BLOCK_WITH_RECEIPTS, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_state_update(&self, block_id: BlockIdOrTag) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(GET_STATE_UPDATE, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(contract_address)?;
        params.insert(key)?;
        params.insert(block_id)?;
        self.send_request::<Value, _>(GET_STORAGE_AT, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_transaction_status(&self, transaction_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(transaction_hash)?;
        self.send_request::<Value, _>(GET_TRANSACTION_STATUS, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_transaction_by_hash(&self, transaction_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(transaction_hash)?;
        self.send_request::<Value, _>(GET_TRANSACTION_BY_HASH, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        index: u64,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(index)?;
        self.send_request::<Value, _>(GET_TRANSACTION_BY_BLOCK_ID_AND_INDEX, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_transaction_receipt(&self, transaction_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(transaction_hash)?;
        self.send_request::<Value, _>(GET_TRANSACTION_RECEIPT, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_class(&self, block_id: BlockIdOrTag, class_hash: ClassHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(class_hash)?;
        self.send_request::<Value, _>(GET_CLASS, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_class_hash_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(contract_address)?;
        self.send_request::<Value, _>(GET_CLASS_HASH_AT, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_class_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(contract_address)?;
        self.send_request::<Value, _>(GET_CLASS_AT, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(GET_BLOCK_TRANSACTION_COUNT, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn call(&self, request: FunctionCall, block_id: BlockIdOrTag) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(request)?;
        params.insert(block_id)?;
        self.send_request::<Value, _>(CALL, params).await.map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn block_number(&self) -> Result<Value> {
        self.send_request::<Value, _>(BLOCK_NUMBER, ArrayParams::new())
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn block_hash_and_number(&self) -> Result<Value> {
        self.send_request::<Value, _>(BLOCK_HASH_AND_NUMBER, ArrayParams::new())
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn chain_id(&self) -> Result<Value> {
        self.send_request::<Value, _>(CHAIN_ID, ArrayParams::new())
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn syncing(&self) -> Result<Value> {
        self.send_request::<Value, _>(SYNCING, ArrayParams::new())
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn get_nonce(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        params.insert(contract_address)?;
        self.send_request::<Value, _>(GET_NONCE, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    // Trace API methods

    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(transaction_hash)?;
        self.send_request::<Value, _>(TRACE_TRANSACTION, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }

    pub async fn trace_block_transactions(&self, block_id: ConfirmedBlockIdOrTag) -> Result<Value> {
        let mut params = ArrayParams::new();
        params.insert(block_id)?;
        self.send_request::<Value, _>(TRACE_BLOCK_TRANSACTIONS, params)
            .await
            .map_err(|e| anyhow!("rpc error: {e}"))
    }
}
