use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use katana_core::service::block_producer::BlockProducer;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_gateway_types::{
    Block, ConfirmedReceipt, ConfirmedTransaction, ContractClass, ErrorCode, GatewayError,
    ReceiptBody, StateUpdate, StateUpdateWithBlock,
};
use katana_pool::TxPool;
use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber};
use katana_primitives::class::{ClassHash, CompiledClass, ContractClassCompilationError};
use katana_provider_api::block::{BlockIdReader, BlockProvider, BlockStatusProvider};
use katana_provider_api::transaction::ReceiptProvider;
use katana_rpc::starknet::StarknetApi;
use katana_rpc_api::error::starknet::StarknetApiError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use starknet::core::types::ResourcePrice;

/// Shared application state containing the backend
#[derive(Clone)]
pub struct AppState {
    pub api: StarknetApi<TxPool, BlockProducer<BlockifierFactory>>,
}

impl AppState {
    // TODO(kariy): support preconfirmed blocks
    async fn get_block(&self, id: BlockIdOrTag) -> Result<Option<Block>, ApiError> {
        self.api
            .on_io_blocking_task(move |this| {
                let provider = this.storage();

                if let Some(num) = provider.convert_block_id(id)? {
                    let block = provider.block(num.into())?.unwrap();
                    let receipts = provider.receipts_by_block(num.into())?.unwrap();
                    let status = provider.block_status(num.into())?.unwrap();

                    let transactions = block
                        .body
                        .into_iter()
                        .map(Into::into)
                        .collect::<Vec<ConfirmedTransaction>>();

                    let transaction_receipts = receipts
                        .into_iter()
                        .zip(transactions.iter())
                        .enumerate()
                        .map(|(index, (receipt, tx))| {
                            let transaction_hash = tx.transaction_hash;
                            let transaction_index = index as u64;
                            let body = ReceiptBody::from(receipt);
                            ConfirmedReceipt { transaction_hash, transaction_index, body }
                        })
                        .collect::<Vec<ConfirmedReceipt>>();

                    let block_hash = block.header.compute_hash();

                    Ok(Some(Block {
                        transactions,
                        transaction_receipts,
                        status: status.into(),
                        block_hash: Some(block_hash),
                        block_number: Some(block.header.number),
                        event_commitment: Some(block.header.events_commitment),
                        l1_da_mode: block.header.l1_da_mode,
                        sequencer_address: Some(block.header.sequencer_address),
                        state_root: Some(block.header.state_root),
                        timestamp: block.header.timestamp,
                        transaction_commitment: Some(block.header.transactions_commitment),
                        state_diff_commitment: Some(block.header.state_diff_commitment),
                        parent_block_hash: block.header.parent_hash,
                        starknet_version: Some(block.header.starknet_version.to_string()),
                        l1_data_gas_price: ResourcePrice {
                            price_in_fri: block.header.l1_data_gas_prices.strk.get().into(),
                            price_in_wei: block.header.l1_data_gas_prices.eth.get().into(),
                        },
                        l1_gas_price: ResourcePrice {
                            price_in_fri: block.header.l1_gas_prices.strk.get().into(),
                            price_in_wei: block.header.l1_gas_prices.eth.get().into(),
                        },
                        l2_gas_price: ResourcePrice {
                            price_in_fri: block.header.l2_gas_prices.strk.get().into(),
                            price_in_wei: block.header.l2_gas_prices.eth.get().into(),
                        },
                    }))
                } else {
                    Ok(None)
                }
            })
            .await?
    }
}

/// Query parameters for block endpoints
#[derive(Debug, Deserialize)]
pub struct BlockIdQuery {
    #[serde(default)]
    #[serde(rename = "blockHash")]
    pub block_hash: Option<BlockHash>,

    #[serde(default)]
    #[serde(rename = "blockNumber")]
    #[serde(deserialize_with = "serde_utils::deserialize_opt_u64")]
    pub block_number: Option<BlockNumber>,
}

impl BlockIdQuery {
    /// Returns the block ID or tag based on the query parameters.
    ///
    /// * If both block hash and block number are provided, an error is returned.
    /// * If neither block hash nor block number are provided, the latest block tag is returned.
    pub fn block_id(self) -> Result<BlockIdOrTag, ApiError> {
        match (self.block_hash, self.block_number) {
            (None, None) => Ok(BlockIdOrTag::Latest),
            (Some(hash), None) => Ok(BlockIdOrTag::Hash(hash)),
            (None, Some(number)) => Ok(BlockIdOrTag::Number(number)),
            (Some(_), Some(_)) => Err(ApiError::gateway_error(
                ErrorCode::MalformedRequest,
                "Cannot specify both block hash and block number",
            )),
        }
    }
}

/// Query parameters for `/get_state_update` endpoint
#[derive(Debug, Deserialize)]
pub struct StateUpdateQuery {
    #[serde(flatten)]
    pub block_query: BlockIdQuery,

    #[serde(default)]
    #[serde(rename = "includeBlock")]
    pub include_block: bool,
}

/// Query parameters for `/get_class_by_hash` and `/get_compiled_class_by_class_hash` endpoints
#[derive(Debug, Deserialize)]
pub struct ClassQuery {
    #[serde(rename = "classHash")]
    pub class_hash: ClassHash,

    #[serde(flatten)]
    pub block_query: BlockIdQuery,
}

/// Handler for `/health` endpoint
///
/// Returns health status of the gateway.
pub async fn health() -> Json<serde_json::Value> {
    Json(json!({"health": true}))
}

/// Handler for `/feeder_gateway/get_block` endpoint
///
/// Returns block information for the specified block.
pub async fn get_block(
    State(state): State<AppState>,
    Query(params): Query<BlockIdQuery>,
) -> Result<Json<Block>, ApiError> {
    let block_id = params.block_id()?;
    let block = state.get_block(block_id).await?.unwrap();
    Ok(Json(block))
}

/// The state update type returns by `/get_state_update` endpoint.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum GetStateUpdateResponse {
    StateUpdate(StateUpdate),
    StateUpdateWithBlock(StateUpdateWithBlock),
}

/// Handler for `/feeder_gateway/get_state_update` endpoint
///
/// Returns state update information for the specified block.
pub async fn get_state_update(
    State(state): State<AppState>,
    Query(params): Query<StateUpdateQuery>,
) -> Result<Json<GetStateUpdateResponse>, ApiError> {
    let include_block = params.include_block;
    let block_id = params.block_query.block_id()?;

    let state_update = state.api.state_update(block_id).await?;
    let state_update = StateUpdate::from(state_update);

    if include_block {
        let block = state.get_block(block_id).await?.expect("qed; should exist");
        let state_update = StateUpdateWithBlock { state_update, block };
        Ok(Json(GetStateUpdateResponse::StateUpdateWithBlock(state_update)))
    } else {
        Ok(Json(GetStateUpdateResponse::StateUpdate(state_update)))
    }
}

/// Handler for `/feeder_gateway/get_class_by_hash` endpoint
///
/// Returns the contract class definition for a given class hash.
pub async fn get_class_by_hash(
    State(state): State<AppState>,
    Query(params): Query<ClassQuery>,
) -> Result<Json<ContractClass>, ApiError> {
    let class_hash = params.class_hash;
    let block_id = params.block_query.block_id()?;
    let class = state.api.class_at_hash(block_id, class_hash).await?;
    Ok(Json(class))
}

/// Handler for `/feeder_gateway/get_compiled_class_by_class_hash` endpoint
///
/// Returns the compiled (CASM) contract class for a given class hash.
pub async fn get_compiled_class_by_class_hash(
    State(state): State<AppState>,
    Query(params): Query<ClassQuery>,
) -> Result<Json<CompiledClass>, ApiError> {
    let class_hash = params.class_hash;
    let block_id = params.block_query.block_id()?;

    state
        .api
        .on_io_blocking_task(move |this| {
            let state = this.state(&block_id)?;
            let Some(class) = state.class(class_hash)? else { todo!() };
            Ok(class.compile()?)
        })
        .await?
        .map(Json)
}

/// API error types with proper HTTP status code mapping
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(untagged)]
pub enum ApiError {
    #[error(transparent)]
    Gateway(#[from] GatewayError),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl ApiError {
    pub fn gateway_error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Gateway(GatewayError { code, message: message.into(), problems: None })
    }

    /// Convert to HTTP status code.
    pub fn status_code(&self) -> StatusCode {
        match self {
            ApiError::Gateway(_) => StatusCode::BAD_REQUEST,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn body(&self) -> Json<Value> {
        Json(json!(self))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = self.body();
        (status, body).into_response()
    }
}

impl From<katana_provider_api::ProviderError> for ApiError {
    fn from(value: katana_provider_api::ProviderError) -> Self {
        ApiError::Internal(value.to_string())
    }
}

impl From<StarknetApiError> for ApiError {
    fn from(value: StarknetApiError) -> Self {
        match GatewayError::try_from(value) {
            Ok(gateway_error) => ApiError::Gateway(gateway_error),
            Err(starknet_error) => ApiError::Internal(starknet_error.to_string()),
        }
    }
}

impl From<ContractClassCompilationError> for ApiError {
    fn from(value: ContractClassCompilationError) -> Self {
        ApiError::Internal(value.to_string())
    }
}
