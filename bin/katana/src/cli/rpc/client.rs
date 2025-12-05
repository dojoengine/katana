use anyhow::{anyhow, Result};
use jsonrpsee::core::client::{ClientT, Error as ClientError};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::{ContractAddress, StorageKey};
use katana_primitives::transaction::TxHash;
use katana_rpc_types::event::EventFilterWithPage;
use katana_rpc_types::trie::ContractStorageKeys;
use katana_rpc_types::FunctionCall;
use serde_json::value::RawValue;
use serde_json::Value;
use url::Url;

/// A JSON-RPC client.
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

    /// Sends a JSON-RPC request.
    ///
    /// ## Arguments
    ///
    /// - `method`: The JSON-RPC method name.
    /// - `params`: The method parameters.
    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// use jsonrpsee::rpc_params;
    /// use serde_json::Value;
    ///
    /// let result: Value = client
    ///     .send_request("starknet_blockNumber", rpc_params!())
    ///     .await?;
    /// ```
    pub async fn send_request<R, Params>(&self, method: &str, params: Params) -> Result<R>
    where
        R: serde::de::DeserializeOwned + Send ,
        Params: ToRpcParams + Send,
    {
        match self.client.request(method, params).await {
            Ok(res) => Ok(res),
            Err(err) => match err {
                ClientError::Call(call_err) => Err(anyhow!(
                    "code={code} message=\"{message}\" data={data}",
                    code = call_err.code(),
                    message = call_err.message(),
                    data = call_err.data().unwrap_or(RawValue::NULL)
                )),
                _ => Err(anyhow!("{err}")),
            },
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Starknet JSON-RPC API
////////////////////////////////////////////////////////////////////////////////

// Starknet JSON-RPC methods name
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
const GET_EVENTS: &str = "starknet_getEvents";
const GET_STORAGE_PROOF: &str = "starknet_getStorageProof";
const TRACE_TRANSACTION: &str = "starknet_traceTransaction";
const TRACE_BLOCK_TRANSACTIONS: &str = "starknet_traceBlockTransactions";

impl Client {
    // Read API methods

    pub async fn spec_version(&self) -> Result<Value> {
        self.send_request(SPEC_VERSION, rpc_params!()).await
    }

    pub async fn get_block_with_tx_hashes(&self, block_id: BlockIdOrTag) -> Result<Value> {
        self.send_request(GET_BLOCK_WITH_TX_HASHES, rpc_params!(block_id)).await
    }

    pub async fn get_block_with_txs(&self, block_id: BlockIdOrTag) -> Result<Value> {
        self.send_request(GET_BLOCK_WITH_TXS, rpc_params!(block_id)).await
    }

    pub async fn get_block_with_receipts(&self, block_id: BlockIdOrTag) -> Result<Value> {
        self.send_request(GET_BLOCK_WITH_RECEIPTS, rpc_params!(block_id)).await
    }

    pub async fn get_state_update(&self, block_id: BlockIdOrTag) -> Result<Value> {
        self.send_request(GET_STATE_UPDATE, rpc_params!(block_id)).await
    }

    pub async fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> Result<Value> {
        self.send_request(GET_STORAGE_AT, rpc_params!(contract_address, key, block_id)).await
    }

    pub async fn get_transaction_status(&self, transaction_hash: TxHash) -> Result<Value> {
        self.send_request(GET_TRANSACTION_STATUS, rpc_params!(transaction_hash)).await
    }

    pub async fn get_transaction_by_hash(&self, transaction_hash: TxHash) -> Result<Value> {
        self.send_request(GET_TRANSACTION_BY_HASH, rpc_params!(transaction_hash)).await
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        index: u64,
    ) -> Result<Value> {
        self.send_request(GET_TRANSACTION_BY_BLOCK_ID_AND_INDEX, rpc_params!(block_id, index)).await
    }

    pub async fn get_transaction_receipt(&self, transaction_hash: TxHash) -> Result<Value> {
        self.send_request(GET_TRANSACTION_RECEIPT, rpc_params!(transaction_hash)).await
    }

    pub async fn get_class(&self, block_id: BlockIdOrTag, class_hash: ClassHash) -> Result<Value> {
        self.send_request(GET_CLASS, rpc_params!(block_id, class_hash)).await
    }

    pub async fn get_class_hash_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Value> {
        self.send_request(GET_CLASS_HASH_AT, rpc_params!(block_id, contract_address)).await
    }

    pub async fn get_class_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Value> {
        self.send_request(GET_CLASS_AT, rpc_params!(block_id, contract_address)).await
    }

    pub async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> Result<Value> {
        self.send_request(GET_BLOCK_TRANSACTION_COUNT, rpc_params!(block_id)).await
    }

    pub async fn call(&self, request: FunctionCall, block_id: BlockIdOrTag) -> Result<Value> {
        self.send_request(CALL, rpc_params!(request, block_id)).await
    }

    pub async fn block_number(&self) -> Result<Value> {
        self.send_request(BLOCK_NUMBER, rpc_params!()).await
    }

    pub async fn block_hash_and_number(&self) -> Result<Value> {
        self.send_request(BLOCK_HASH_AND_NUMBER, rpc_params!()).await
    }

    pub async fn chain_id(&self) -> Result<Value> {
        self.send_request(CHAIN_ID, rpc_params!()).await
    }

    pub async fn syncing(&self) -> Result<Value> {
        self.send_request(SYNCING, rpc_params!()).await
    }

    pub async fn get_nonce(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> Result<Value> {
        self.send_request(GET_NONCE, rpc_params!(block_id, contract_address)).await
    }

    pub async fn get_events(&self, filter: EventFilterWithPage) -> Result<Value> {
        self.send_request(GET_EVENTS, rpc_params!(filter)).await
    }

    // Trace API methods

    pub async fn trace_transaction(&self, transaction_hash: TxHash) -> Result<Value> {
        self.send_request(TRACE_TRANSACTION, rpc_params!(transaction_hash)).await
    }

    pub async fn trace_block_transactions(&self, block_id: ConfirmedBlockIdOrTag) -> Result<Value> {
        self.send_request(TRACE_BLOCK_TRANSACTIONS, rpc_params!(block_id)).await
    }

    pub async fn get_storage_proof(
        &self,
        block_id: BlockIdOrTag,
        class_hashes: Option<Vec<ClassHash>>,
        contract_addresses: Option<Vec<ContractAddress>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeys>>,
    ) -> Result<Value> {
        self.send_request(
            GET_STORAGE_PROOF,
            rpc_params!(block_id, class_hashes, contract_addresses, contracts_storage_keys),
        )
        .await
    }
}
