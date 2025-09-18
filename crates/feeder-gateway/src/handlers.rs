use std::collections::HashMap;
use std::sync::Arc;

use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_primitives::class::CasmContractClass;
use katana_primitives::Felt;
use serde_json::json;

use crate::types::{Block, BlockId, ContractClass};

/// Handler for `/feeder_gateway/get_block` endpoint
///
/// Returns block information for the specified block.
///
/// ## Parameters
/// - `block_id`: Block identifier (hash, number, or latest)
///
/// ## Response
/// Returns a `Block` object containing:
/// - Block metadata (hash, number, timestamp, etc.)
/// - All transactions in the block with receipts
/// - Gas prices and commitments
pub async fn get_block(
    backend: Arc<Backend<BlockifierFactory>>,
    block_id: BlockId,
) -> Result<Block, HandlerError> {
    // TODO: Implement block retrieval
    // 1. Resolve block_id to actual block number/hash
    // 2. Fetch block from backend storage
    // 3. Convert to feeder gateway Block format
    // 4. Include transactions and receipts
    
    let _ = backend; // Suppress unused warning
    let _ = block_id;
    
    Err(HandlerError::NotImplemented("get_block".to_string()))
}

/// Handler for `/feeder_gateway/get_state_update` endpoint
///
/// Returns state update information for the specified block.
///
/// ## Parameters
/// - `block_id`: Block identifier (hash, number, or latest)
/// - `include_block`: If true, also includes block data in response
///
/// ## Response
/// Returns either:
/// - `StateUpdate`: Just the state changes
/// - `StateUpdateWithBlock`: State changes + block data (if include_block=true)
pub async fn get_state_update(
    backend: Arc<Backend<BlockifierFactory>>,
    block_id: BlockId,
    include_block: bool,
) -> Result<serde_json::Value, HandlerError> {
    // TODO: Implement state update retrieval
    // 1. Resolve block_id to actual block
    // 2. Fetch state diff from backend
    // 3. Build StateDiff with:
    //    - storage_diffs: changed storage slots
    //    - deployed_contracts: new contract deployments  
    //    - declared_classes: new class declarations
    //    - nonces: changed nonces
    //    - replaced_classes: class replacements
    // 4. If include_block=true, also fetch block data
    
    let _ = backend; // Suppress unused warning
    let _ = block_id;
    let _ = include_block;
    
    Err(HandlerError::NotImplemented("get_state_update".to_string()))
}

/// Handler for `/feeder_gateway/get_class_by_hash` endpoint
///
/// Returns the contract class definition for a given class hash.
///
/// ## Parameters  
/// - `class_hash`: Hash of the class to retrieve
/// - `block_id`: Block at which to fetch the class (for historical queries)
///
/// ## Response
/// Returns a `ContractClass` containing:
/// - Sierra program (for Cairo 1+ contracts)
/// - ABI definition  
/// - Entry points
/// - Or legacy Cairo 0 class format
pub async fn get_class_by_hash(
    backend: Arc<Backend<BlockifierFactory>>,
    class_hash: Felt,
    block_id: BlockId,
) -> Result<ContractClass, HandlerError> {
    // TODO: Implement class retrieval
    // 1. Resolve block_id to get the proper state view
    // 2. Look up class definition in storage
    // 3. Convert from internal format to RPC ContractClass format
    // 4. Handle both Cairo 0 and Cairo 1+ formats
    
    let _ = backend; // Suppress unused warning
    let _ = class_hash;
    let _ = block_id;
    
    Err(HandlerError::NotImplemented("get_class_by_hash".to_string()))
}

/// Handler for `/feeder_gateway/get_compiled_class_by_class_hash` endpoint
///
/// Returns the compiled (CASM) contract class for a given class hash.
///
/// ## Parameters
/// - `class_hash`: Hash of the class to retrieve compiled version for
/// - `block_id`: Block at which to fetch the class (for historical queries)  
///
/// ## Response
/// Returns a `CasmContractClass` containing:
/// - Compiled CASM bytecode
/// - Entry points with PC offsets
/// - Hints for Cairo runner
pub async fn get_compiled_class_by_class_hash(
    backend: Arc<Backend<BlockifierFactory>>,
    class_hash: Felt,
    block_id: BlockId,
) -> Result<CasmContractClass, HandlerError> {
    // TODO: Implement compiled class retrieval
    // 1. Resolve block_id to get proper state view
    // 2. Look up compiled class in storage
    // 3. Return CASM format (only for Cairo 1+ classes)
    // 4. Handle case where class is Cairo 0 (no CASM equivalent)
    
    let _ = backend; // Suppress unused warning
    let _ = class_hash;
    let _ = block_id;
    
    Err(HandlerError::NotImplemented("get_compiled_class_by_class_hash".to_string()))
}

/// Parse query parameters from URL query string
pub fn parse_query_params(query: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            params.insert(
                urlencoding::decode(key).unwrap_or_default().to_string(),
                urlencoding::decode(value).unwrap_or_default().to_string(),
            );
        }
    }
    
    params
}

/// Parse block_id from query parameters
pub fn parse_block_id(params: &HashMap<String, String>) -> Result<BlockId, HandlerError> {
    match (params.get("blockHash"), params.get("blockNumber")) {
        (Some(hash_str), None) => {
            let hash = Felt::from_hex(hash_str)
                .map_err(|_| HandlerError::InvalidParameter("Invalid block hash format".to_string()))?;
            Ok(BlockId::Hash(hash))
        }
        (None, Some(number_str)) => {
            let number = number_str.parse::<u64>()
                .map_err(|_| HandlerError::InvalidParameter("Invalid block number format".to_string()))?;
            Ok(BlockId::Number(number))
        }
        (None, None) => Ok(BlockId::Latest),
        (Some(_), Some(_)) => Err(HandlerError::InvalidParameter(
            "Cannot specify both blockHash and blockNumber".to_string()
        )),
    }
}

/// Parse class_hash from query parameters
pub fn parse_class_hash(params: &HashMap<String, String>) -> Result<Felt, HandlerError> {
    let hash_str = params.get("classHash")
        .ok_or_else(|| HandlerError::InvalidParameter("Missing required parameter: classHash".to_string()))?;
    
    Felt::from_hex(hash_str)
        .map_err(|_| HandlerError::InvalidParameter("Invalid class hash format".to_string()))
}

/// Check if includeBlock parameter is set to true
pub fn should_include_block(params: &HashMap<String, String>) -> bool {
    params.get("includeBlock")
        .map(|value| value.to_lowercase() == "true")
        .unwrap_or(false)
}

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
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

impl HandlerError {
    /// Convert to HTTP status code
    pub fn status_code(&self) -> u16 {
        match self {
            HandlerError::NotImplemented(_) => 501, // Not Implemented
            HandlerError::InvalidParameter(_) => 400, // Bad Request
            HandlerError::BlockNotFound | HandlerError::ClassNotFound => 404, // Not Found
            HandlerError::Internal(_) => 500, // Internal Server Error
        }
    }

    /// Convert to JSON error response
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "error": self.to_string(),
            "code": self.status_code()
        })
    }
}
