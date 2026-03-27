use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::Felt;
use katana_rpc_types::trie::Nodes;
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
    pub prev_block_number: Option<BlockNumber>,

    /// The number of the attested block.
    pub block_number: BlockNumber,

    /// The block number Katana forked from (if running in fork mode).
    /// Attested by TEE hardware via report_data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_block_number: Option<BlockNumber>,

    /// Merkle root of all events in the attested block.
    /// Included in report_data: Poseidon(state_root, block_hash, fork_block, events_commitment).
    pub events_commitment: Felt,

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

/// Response type for event inclusion proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventProofResponse {
    /// The block number containing the event.
    pub block_number: BlockNumber,

    /// Merkle root of all events in the block (from block header).
    pub events_commitment: Felt,

    /// Total number of events in the block.
    pub events_count: u32,

    /// Poseidon hash of the event: H(tx_hash, from_address, H(keys), H(data)).
    pub event_hash: Felt,

    /// Index of the event in the block's flattened events list.
    pub event_index: u32,

    /// Merkle-Patricia trie proof nodes (same format as storage proofs).
    pub merkle_proof: Nodes,

    /// Transaction hash that emitted the event.
    pub tx_hash: Felt,

    /// Address of the contract that emitted the event.
    pub from_address: Felt,

    /// Event keys.
    pub keys: Vec<Felt>,

    /// Event data.
    pub data: Vec<Felt>,
}

/// TEE API for generating hardware attestation quotes.
///
/// This API allows clients to request attestation quotes that
/// cryptographically bind the current blockchain state to a
/// hardware-backed measurement.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "tee"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "tee"))]
pub trait TeeApi {
    /// Generate a TEE attestation quote for the requested block state.
    ///
    /// The quote includes a commitment to the requested block's state root
    /// and block hash, allowing verifiers to cryptographically verify
    /// that the state was attested from within a trusted execution environment.
    ///
    /// `prev_block_id` is optional and included in the response for transition-style flows.
    #[method(name = "generateQuote")]
    async fn generate_quote(
        &self,
        prev_block_id: Option<BlockNumber>,
        block_id: BlockNumber,
    ) -> RpcResult<TeeQuoteResponse>;

    /// Get a Merkle inclusion proof for a specific event in a block.
    ///
    /// Returns a proof that event at `event_index` is included in the block's
    /// `events_commitment` (Merkle root). The `events_commitment` is bound to the
    /// TEE attestation via `report_data`, so this proof chain connects an individual
    /// event to the hardware attestation.
    #[method(name = "getEventProof")]
    async fn get_event_proof(
        &self,
        block_number: BlockNumber,
        event_index: u32,
    ) -> RpcResult<EventProofResponse>;
}
