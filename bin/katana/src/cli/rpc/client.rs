use anyhow::{anyhow, Result};
use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_rpc_api::error::starknet::StarknetApiError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// A JSON-RPC client for Starknet that returns raw JSON responses.
/// This is primarily used for debugging and validating RPC server responses.
#[derive(Debug, Clone)]
pub struct Client {
    client: reqwest::Client,
    url: Url,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    method: &'static str,
    params: Value,
    id: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

// JSON-RPC method names
const SPEC_VERSION: &str = "starknet_specVersion";
const GET_BLOCK_WITH_TX_HASHES: &str = "starknet_getBlockWithTxHashes";
const GET_BLOCK_WITH_TXS: &str = "starknet_getBlockWithTxs";
const GET_BLOCK_WITH_RECEIPTS: &str = "starknet_getBlockWithReceipts";
const GET_STATE_UPDATE: &str = "starknet_getStateUpdate";
const GET_STORAGE_AT: &str = "starknet_getStorageAt";
const GET_TRANSACTION_BY_HASH: &str = "starknet_getTransactionByHash";
const GET_TRANSACTION_BY_BLOCK_ID_AND_INDEX: &str = "starknet_getTransactionByBlockIdAndIndex";
const GET_TRANSACTION_RECEIPT: &str = "starknet_getTransactionReceipt";
const GET_TRANSACTION_STATUS: &str = "starknet_getTransactionStatus";
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

impl Client {
    pub fn new(url: Url) -> Self {
        Self { client: reqwest::Client::new(), url }
    }

    async fn send_request(&self, method: &'static str, params: Value) -> Result<Value> {
        let request = JsonRpcRequest { jsonrpc: "2.0", method, params, id: 1 };

        let response = self
            .client
            .post(self.url.clone())
            .json(&request)
            .send()
            .await
            .map_err(|e| anyhow!("HTTP request failed: {e}"))?;

        let json_response: JsonRpcResponse =
            response.json().await.map_err(|e| anyhow!("Failed to parse JSON response: {e}"))?;

        if let Some(error) = json_response.error {
            let starknet_error = convert_to_starknet_error(error);
            return Err(anyhow!("RPC error: {}", starknet_error));
        }

        json_response.result.ok_or_else(|| anyhow!("No result in response"))
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Client Starknet JSON-RPC implementations
////////////////////////////////////////////////////////////////////////////////////////////////////

impl Client {
    // Read API methods

    pub async fn spec_version(&self) -> Result<Value> {
        self.send_request(SPEC_VERSION, serde_json::json!([])).await
    }

    pub async fn get_block_with_tx_hashes(&self, block_id: Value) -> Result<Value> {
        self.send_request(GET_BLOCK_WITH_TX_HASHES, serde_json::json!([block_id])).await
    }

    pub async fn get_block_with_txs(&self, block_id: Value) -> Result<Value> {
        self.send_request(GET_BLOCK_WITH_TXS, serde_json::json!([block_id])).await
    }

    pub async fn get_block_with_receipts(&self, block_id: Value) -> Result<Value> {
        self.send_request(GET_BLOCK_WITH_RECEIPTS, serde_json::json!([block_id])).await
    }

    pub async fn get_state_update(&self, block_id: Value) -> Result<Value> {
        self.send_request(GET_STATE_UPDATE, serde_json::json!([block_id])).await
    }

    pub async fn get_storage_at(
        &self,
        contract_address: Felt,
        key: Felt,
        block_id: Value,
    ) -> Result<Value> {
        self.send_request(
            GET_STORAGE_AT,
            serde_json::json!([
                format!("{:#x}", contract_address),
                format!("{:#x}", key),
                block_id
            ]),
        )
        .await
    }

    pub async fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Result<Value> {
        self.send_request(GET_TRANSACTION_BY_HASH, serde_json::json!([format!("{:#x}", tx_hash)]))
            .await
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: Value,
        index: u64,
    ) -> Result<Value> {
        self.send_request(
            GET_TRANSACTION_BY_BLOCK_ID_AND_INDEX,
            serde_json::json!([block_id, index]),
        )
        .await
    }

    pub async fn get_transaction_receipt(&self, tx_hash: TxHash) -> Result<Value> {
        self.send_request(GET_TRANSACTION_RECEIPT, serde_json::json!([format!("{:#x}", tx_hash)]))
            .await
    }

    pub async fn get_transaction_status(&self, tx_hash: TxHash) -> Result<Value> {
        self.send_request(GET_TRANSACTION_STATUS, serde_json::json!([format!("{:#x}", tx_hash)]))
            .await
    }

    pub async fn get_class(&self, block_id: Value, class_hash: Felt) -> Result<Value> {
        self.send_request(GET_CLASS, serde_json::json!([block_id, format!("{:#x}", class_hash)]))
            .await
    }

    pub async fn get_class_hash_at(
        &self,
        block_id: Value,
        contract_address: Felt,
    ) -> Result<Value> {
        self.send_request(
            GET_CLASS_HASH_AT,
            serde_json::json!([block_id, format!("{:#x}", contract_address)]),
        )
        .await
    }

    pub async fn get_class_at(&self, block_id: Value, contract_address: Felt) -> Result<Value> {
        self.send_request(
            GET_CLASS_AT,
            serde_json::json!([block_id, format!("{:#x}", contract_address)]),
        )
        .await
    }

    pub async fn get_block_transaction_count(&self, block_id: Value) -> Result<Value> {
        self.send_request(GET_BLOCK_TRANSACTION_COUNT, serde_json::json!([block_id])).await
    }

    pub async fn call(&self, request: Value, block_id: Value) -> Result<Value> {
        self.send_request(CALL, serde_json::json!([request, block_id])).await
    }

    pub async fn block_number(&self) -> Result<Value> {
        self.send_request(BLOCK_NUMBER, serde_json::json!([])).await
    }

    pub async fn block_hash_and_number(&self) -> Result<Value> {
        self.send_request(BLOCK_HASH_AND_NUMBER, serde_json::json!([])).await
    }

    pub async fn chain_id(&self) -> Result<Value> {
        self.send_request(CHAIN_ID, serde_json::json!([])).await
    }

    pub async fn syncing(&self) -> Result<Value> {
        self.send_request(SYNCING, serde_json::json!([])).await
    }

    pub async fn get_nonce(&self, block_id: Value, contract_address: Felt) -> Result<Value> {
        self.send_request(
            GET_NONCE,
            serde_json::json!([block_id, format!("{:#x}", contract_address)]),
        )
        .await
    }

    // Trace API methods

    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<Value> {
        self.send_request(
            TRACE_TRANSACTION,
            serde_json::json!([format!("{:#x}", transaction_hash)]),
        )
        .await
    }

    pub async fn trace_block_transactions(&self, block_id: Value) -> Result<Value> {
        self.send_request(TRACE_BLOCK_TRANSACTIONS, serde_json::json!([block_id])).await
    }
}

/// Convert a JSON-RPC error to a StarknetApiError
fn convert_to_starknet_error(error: JsonRpcError) -> StarknetApiError {
    match error.code {
        1 => StarknetApiError::FailedToReceiveTxn,
        20 => StarknetApiError::ContractNotFound,
        21 => StarknetApiError::EntrypointNotFound,
        22 => StarknetApiError::InvalidCallData,
        24 => StarknetApiError::BlockNotFound,
        27 => StarknetApiError::InvalidTxnIndex,
        28 => StarknetApiError::ClassHashNotFound,
        29 => StarknetApiError::TxnHashNotFound,
        32 => StarknetApiError::NoBlocks,
        33 => StarknetApiError::InvalidContinuationToken,
        34 => StarknetApiError::TooManyKeysInFilter,
        38 => StarknetApiError::FailedToFetchPendingTransactions,
        50 => StarknetApiError::InvalidContractClass,
        51 => StarknetApiError::ClassAlreadyDeclared,
        54 => StarknetApiError::InsufficientAccountBalance,
        57 => StarknetApiError::ContractClassSizeIsTooLarge,
        58 => StarknetApiError::NonAccount,
        59 => StarknetApiError::DuplicateTransaction,
        60 => StarknetApiError::CompiledClassHashMismatch,
        61 => StarknetApiError::UnsupportedTransactionVersion,
        62 => StarknetApiError::UnsupportedContractClassVersion,
        64 => StarknetApiError::ReplacementTransactionUnderpriced,
        65 => StarknetApiError::FeeBelowMinimum,
        66 => StarknetApiError::InvalidSubscriptionId,
        67 => StarknetApiError::TooManyAddressesInFilter,
        68 => StarknetApiError::TooManyBlocksBack,
        _ => StarknetApiError::unexpected(error.message),
    }
}
