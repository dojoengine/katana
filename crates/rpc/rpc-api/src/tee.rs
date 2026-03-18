use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

/// A L2→L1 message emitted by a contract execution.
///
/// Fields match `MessageToL1` in primitives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeeL2ToL1Message {
    /// L2 contract that sent the message.
    pub from_address: Felt,
    /// L1 contract address the message is directed to.
    pub to_address: Felt,
    /// Message payload.
    pub payload: Vec<Felt>,
}

/// A L1→L2 message derived from an L1Handler transaction.
///
/// All fields are required to independently recompute the `message_hash`:
/// `keccak256(from_address_u256, to_address, nonce, selector, payload.len, payload...)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeeL1ToL2Message {
    /// Ethereum address of the L1 sender (padded to felt).
    pub from_address: Felt,
    /// L2 contract address (the L1Handler target).
    pub to_address: Felt,
    /// Entry point selector of the L1Handler function.
    pub selector: Felt,
    /// Message payload (excludes the prepended from_address in calldata).
    pub payload: Vec<Felt>,
    /// Message nonce assigned by the core contract on L1.
    pub nonce: Felt,
}

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

    /// Poseidon commitment over all L1<->L2 messages from prev_block+1 to block_number.
    ///
    /// Computed as `Poseidon(l2_to_l1_commitment, l1_to_l2_commitment)` where each direction's
    /// commitment is `Poseidon` over the individual message hashes in that range.
    pub messages_commitment: Felt,

    /// All L2→L1 messages emitted in the attested block range.
    pub l2_to_l1_messages: Vec<TeeL2ToL1Message>,

    /// All L1→L2 messages processed in the attested block range.
    pub l1_to_l2_messages: Vec<TeeL1ToL2Message>,
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
    async fn generate_quote(
        &self,
        prev_block_id: Option<BlockNumber>,
        block_id: BlockNumber,
    ) -> RpcResult<TeeQuoteResponse>;
}
