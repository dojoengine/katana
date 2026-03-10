use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

/// Response type for TEE quote generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeeQuoteResponse {
    /// The raw attestation quote bytes (hex-encoded).
    pub quote: String,

    /// The prev state root of the attested block.
    pub prev_state_root: Felt,

    /// The state root at the attested block.
    pub state_root: Felt,

    /// The hash of the previous block.
    pub prev_block_hash: BlockHash,

    /// The hash of the attested block.
    pub block_hash: BlockHash,

    /// The number of the previous block.
    pub prev_block_number: Felt,
    
    /// The number of the attested block.
    pub block_number: Felt,
}

/// TEE API for generating hardware attestation quotes.
///
/// This API allows clients to request attestation quotes that
/// cryptographically bind the current blockchain state to a
/// hardware-backed measurement.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "tee"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "tee"))]
pub trait TeeApi {
    /// Generate a TEE attestation quote for the current blockchain state.
    ///
    /// The quote includes a commitment to the latest block's state root
    /// and block hash, allowing verifiers to cryptographically verify
    /// that the state was attested from within a trusted execution environment.
    ///
    /// # Returns
    /// - `TeeQuoteResponse` containing the quote and the attested state information.
    ///
    /// # Errors
    /// - Returns an error if TEE quote generation fails or TEE is not available.
    #[method(name = "generateQuote")]
    async fn generate_quote(&self, prev_block_id: Option<BlockNumber>, block_id: BlockNumber) -> RpcResult<TeeQuoteResponse>;
}
