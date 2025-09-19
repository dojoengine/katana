use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_primitives::Felt;
use serde::Deserialize;
use serde_json::json;

use crate::types::BlockId;

/// Shared application state containing the backend
#[derive(Clone)]
pub struct AppState {
    pub backend: Arc<Backend<BlockifierFactory>>,
}

/// Query parameters for block endpoints
#[derive(Debug, Deserialize)]
pub struct BlockQuery {
    #[serde(rename = "blockHash")]
    pub block_hash: Option<String>,
    #[serde(rename = "blockNumber")]
    pub block_number: Option<u64>,
}

/// Query parameters for state update endpoint  
#[derive(Debug, Deserialize)]
pub struct StateUpdateQuery {
    #[serde(rename = "blockHash")]
    pub block_hash: Option<String>,
    #[serde(rename = "blockNumber")]
    pub block_number: Option<u64>,
    #[serde(rename = "includeBlock")]
    pub include_block: Option<String>,
}

/// Query parameters for class endpoints
#[derive(Debug, Deserialize)]
pub struct ClassQuery {
    #[serde(rename = "classHash")]
    pub class_hash: String,
    #[serde(rename = "blockHash")]
    pub block_hash: Option<String>,
    #[serde(rename = "blockNumber")]
    pub block_number: Option<u64>,
}

/// Handler for `/feeder_gateway/get_block` endpoint
///
/// Returns block information for the specified block.
pub async fn get_block(
    State(state): State<AppState>,
    Query(params): Query<BlockQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let block_id = parse_block_id_from_query(&params.block_hash, params.block_number)?;

    // TODO: Implement actual block retrieval
    // 1. Resolve block_id to actual block number/hash
    // 2. Fetch block from backend.blockchain
    // 3. Convert to feeder gateway Block format
    // 4. Include transactions and receipts

    let _backend = state.backend;
    let _block_id = block_id;

    Err(ApiError::NotImplemented("get_block endpoint not yet implemented".to_string()))
}

/// Handler for `/feeder_gateway/get_state_update` endpoint  
///
/// Returns state update information for the specified block.
pub async fn get_state_update(
    State(state): State<AppState>,
    Query(params): Query<StateUpdateQuery>,
) -> Result<Response, ApiError> {
    let block_id = parse_block_id_from_query(&params.block_hash, params.block_number)?;
    let include_block = params.include_block.as_deref() == Some("true");

    // TODO: Implement actual state update retrieval
    // 1. Resolve block_id to actual block
    // 2. Fetch state diff from backend
    // 3. Build StateDiff with storage changes, deployments, etc.
    // 4. If include_block=true, also fetch block data

    let _backend = state.backend;
    let _block_id = block_id;
    let _include_block = include_block;

    Err(ApiError::NotImplemented("get_state_update endpoint not yet implemented".to_string()))
}

/// Handler for `/feeder_gateway/get_class_by_hash` endpoint
///
/// Returns the contract class definition for a given class hash.
pub async fn get_class_by_hash(
    State(state): State<AppState>,
    Query(params): Query<ClassQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let class_hash = parse_class_hash(&params.class_hash)?;
    let block_id = parse_block_id_from_query(&params.block_hash, params.block_number)?;

    // TODO: Implement actual class retrieval
    // 1. Resolve block_id to get proper state view
    // 2. Look up class definition in storage
    // 3. Convert from internal format to RPC ContractClass format
    // 4. Handle both Cairo 0 and Cairo 1+ formats

    let _backend = state.backend;
    let _class_hash = class_hash;
    let _block_id = block_id;

    Err(ApiError::NotImplemented("get_class_by_hash endpoint not yet implemented".to_string()))
}

/// Handler for `/feeder_gateway/get_compiled_class_by_class_hash` endpoint
///
/// Returns the compiled (CASM) contract class for a given class hash.
pub async fn get_compiled_class_by_class_hash(
    State(state): State<AppState>,
    Query(params): Query<ClassQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let class_hash = parse_class_hash(&params.class_hash)?;
    let block_id = parse_block_id_from_query(&params.block_hash, params.block_number)?;

    // TODO: Implement actual compiled class retrieval
    // 1. Resolve block_id to get proper state view
    // 2. Look up compiled class in storage
    // 3. Return CASM format (only for Cairo 1+ classes)
    // 4. Handle case where class is Cairo 0 (no CASM equivalent)

    let _backend = state.backend;
    let _class_hash = class_hash;
    let _block_id = block_id;

    Err(ApiError::NotImplemented(
        "get_compiled_class_by_class_hash endpoint not yet implemented".to_string(),
    ))
}

/// Parse block_id from optional query parameters  
fn parse_block_id_from_query(
    block_hash: &Option<String>,
    block_number: Option<u64>,
) -> Result<BlockId, ApiError> {
    match (block_hash, block_number) {
        (Some(hash_str), None) => {
            let hash = Felt::from_hex(hash_str)
                .map_err(|_| ApiError::InvalidParameter("Invalid block hash format".to_string()))?;
            Ok(BlockId::Hash(hash))
        }
        (None, Some(number)) => Ok(BlockId::Number(number)),
        (None, None) => Ok(BlockId::Latest),
        (Some(_), Some(_)) => Err(ApiError::InvalidParameter(
            "Cannot specify both blockHash and blockNumber".to_string(),
        )),
    }
}

/// Parse class_hash from string parameter
fn parse_class_hash(class_hash_str: &str) -> Result<Felt, ApiError> {
    Felt::from_hex(class_hash_str)
        .map_err(|_| ApiError::InvalidParameter("Invalid class hash format".to_string()))
}

/// API error types with proper HTTP status code mapping
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Block not found")]
    BlockNotFound,

    #[error("Class not found")]
    ClassNotFound,

    #[error("Internal server error: {0}")]
    Internal(String),
}

impl ApiError {
    /// Convert to HTTP status code
    pub fn status_code(&self) -> StatusCode {
        match self {
            ApiError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
            ApiError::InvalidParameter(_) => StatusCode::BAD_REQUEST,
            ApiError::BlockNotFound | ApiError::ClassNotFound => StatusCode::NOT_FOUND,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = Json(json!({
            "error": self.to_string(),
            "code": status.as_u16()
        }));

        (status, body).into_response()
    }
}
