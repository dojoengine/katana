use anyhow::{anyhow, Result};
use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_utils::node::StarknetError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use starknet::core::types::requests::*;
use starknet::core::types::{BlockHashAndNumber, BlockId, FunctionCall, SyncStatusType};
use starknet::providers::jsonrpc::{
    HttpTransport, JsonRpcClientError, JsonRpcMethod as StarknetJsonRpcMethod, JsonRpcResponse,
    JsonRpcTransport,
};
use starknet::providers::{ProviderError as StarknetProviderError, Url};

/// A generic JSON-RPC client with any transport.
///
/// A "transport" is any implementation that can send JSON-RPC requests and receive responses. This
/// most commonly happens over a network via HTTP connections, as with [`HttpTransport`].
#[derive(Debug, Clone)]
pub struct Client {
    transport: HttpTransport,
}

impl Client {
    pub fn new(url: Url) -> Self {
        Self { transport: HttpTransport::new(url) }
    }

    async fn send_request<P, R>(
        &self,
        method: StarknetJsonRpcMethod,
        params: P,
    ) -> Result<R, StarknetProviderError>
    where
        P: Serialize + Send + Sync,
        R: DeserializeOwned,
    {
        match self
            .transport
            .send_request(method, params)
            .await
            .map_err(JsonRpcClientError::TransportError)?
        {
            JsonRpcResponse::Success { result, .. } => Ok(result),
            JsonRpcResponse::Error { error, .. } => {
                Err(match TryInto::<StarknetError>::try_into(&error) {
                    Ok(error) => StarknetProviderError::StarknetError(error),
                    Err(_) => JsonRpcClientError::<<HttpTransport as JsonRpcTransport>::Error>::JsonRpcError(error).into(),
                })
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Client Starknet JSON-RPC implementations
////////////////////////////////////////////////////////////////////////////////////////////////////

impl Client {
    // Read API methods

    pub async fn spec_version(&self) -> Result<Value> {
        self.send_request(StarknetJsonRpcMethod::SpecVersion, SpecVersionRequest)
            .await
            .map_err(|e| anyhow!("Failed to get spec version: {e}"))
    }

    pub async fn get_block_with_tx_hashes(&self, block_id: BlockId) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetBlockWithTxHashes,
            GetBlockWithTxHashesRequestRef { block_id: block_id.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get block with tx hashes: {e}"))
    }

    pub async fn get_block_with_txs(&self, block_id: BlockId) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetBlockWithTxs,
            GetBlockWithTxsRequestRef { block_id: block_id.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get block with txs: {e}"))
    }

    pub async fn get_block_with_receipts(&self, block_id: BlockId) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetBlockWithReceipts,
            GetBlockWithReceiptsRequestRef { block_id: block_id.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get block with receipts: {e}"))
    }

    pub async fn get_state_update(&self, block_id: BlockId) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetStateUpdate,
            GetStateUpdateRequestRef { block_id: block_id.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get state update: {e}"))
    }

    pub async fn get_storage_at(
        &self,
        contract_address: Felt,
        key: Felt,
        block_id: BlockId,
    ) -> Result<Felt> {
        self.send_request(
            StarknetJsonRpcMethod::GetStorageAt,
            GetStorageAtRequestRef {
                contract_address: contract_address.as_ref(),
                block_id: block_id.as_ref(),
                key: key.as_ref(),
            },
        )
        .await
        .map_err(|e| anyhow!("Failed to get storage at: {e}"))
    }

    pub async fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetTransactionByHash,
            GetTransactionByHashRequestRef { transaction_hash: tx_hash.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get transaction by hash: {e}"))
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockId,
        index: u64,
    ) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetTransactionByBlockIdAndIndex,
            GetTransactionByBlockIdAndIndexRequestRef {
                block_id: block_id.as_ref(),
                index: &index,
            },
        )
        .await
        .map_err(|e| anyhow!("Failed to get transaction by block id and index: {e}"))
    }

    pub async fn get_transaction_receipt(&self, tx_hash: TxHash) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetTransactionReceipt,
            GetTransactionReceiptRequestRef { transaction_hash: tx_hash.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get transaction receipt: {e}"))
    }

    pub async fn get_transaction_status(&self, tx_hash: TxHash) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetTransactionStatus,
            GetTransactionStatusRequestRef { transaction_hash: tx_hash.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get transaction status: {e}"))
    }

    pub async fn get_class(&self, block_id: BlockId, class_hash: Felt) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetClass,
            GetClassRequestRef { block_id: block_id.as_ref(), class_hash: class_hash.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get class: {e}"))
    }

    pub async fn get_class_hash_at(
        &self,
        block_id: BlockId,
        contract_address: Felt,
    ) -> Result<Felt> {
        self.send_request(
            StarknetJsonRpcMethod::GetClassHashAt,
            GetClassHashAtRequestRef {
                block_id: block_id.as_ref(),
                contract_address: contract_address.as_ref(),
            },
        )
        .await
        .map_err(|e| anyhow!("Failed to get class hash at: {e}"))
    }

    pub async fn get_class_at(&self, block_id: BlockId, contract_address: Felt) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::GetClassAt,
            GetClassAtRequestRef {
                block_id: block_id.as_ref(),
                contract_address: contract_address.as_ref(),
            },
        )
        .await
        .map_err(|e| anyhow!("Failed to get class at: {e}"))
    }

    pub async fn get_block_transaction_count(&self, block_id: BlockId) -> Result<u64> {
        self.send_request(
            StarknetJsonRpcMethod::GetBlockTransactionCount,
            GetBlockTransactionCountRequestRef { block_id: block_id.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to get block transaction count: {e}"))
    }

    pub async fn call(&self, request: FunctionCall, block_id: BlockId) -> Result<Vec<Felt>> {
        self.send_request(
            StarknetJsonRpcMethod::Call,
            CallRequestRef { request: request.as_ref(), block_id: block_id.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to call: {e}"))
    }

    pub async fn block_number(&self) -> Result<BlockNumber> {
        self.send_request(StarknetJsonRpcMethod::BlockNumber, BlockNumberRequest)
            .await
            .map_err(|e| anyhow!("Failed to get block number: {e}"))
    }

    pub async fn block_hash_and_number(&self) -> Result<BlockHashAndNumber> {
        self.send_request(StarknetJsonRpcMethod::BlockHashAndNumber, BlockHashAndNumberRequest)
            .await
            .map_err(|e| anyhow!("Failed to get block hash and number: {e}"))
    }

    pub async fn chain_id(&self) -> Result<Felt> {
        self.send_request(StarknetJsonRpcMethod::ChainId, ChainIdRequest)
            .await
            .map_err(|e| anyhow!("Failed to get chain id: {e}"))
    }

    pub async fn syncing(&self) -> Result<SyncStatusType> {
        self.send_request(StarknetJsonRpcMethod::Syncing, SyncingRequest)
            .await
            .map_err(|e| anyhow!("Failed to get syncing status: {e}"))
    }

    pub async fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> Result<Felt> {
        self.send_request(
            StarknetJsonRpcMethod::GetNonce,
            GetNonceRequestRef {
                block_id: block_id.as_ref(),
                contract_address: contract_address.as_ref(),
            },
        )
        .await
        .map_err(|e| anyhow!("Failed to get nonce: {e}"))
    }

    // Trace API methods

    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<Value> {
        self.send_request(
            StarknetJsonRpcMethod::TraceTransaction,
            TraceTransactionRequestRef { transaction_hash: transaction_hash.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to trace transaction: {e}"))
    }

    pub async fn trace_block_transactions(&self, block_id: BlockId) -> Result<Vec<Value>> {
        self.send_request(
            StarknetJsonRpcMethod::TraceBlockTransactions,
            TraceBlockTransactionsRequestRef { block_id: block_id.as_ref() },
        )
        .await
        .map_err(|e| anyhow!("Failed to trace block transactions: {e}"))
    }
}
