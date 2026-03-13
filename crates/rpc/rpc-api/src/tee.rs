use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::Felt;
use katana_rpc_types::trie::Nodes;
use serde::{Deserialize, Serialize};

/// Response type for TEE quote generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeeQuoteResponse {
    /// The raw attestation quote bytes (hex-encoded).
    pub quote: String,

    /// The state root at the previous block (or zero when not provided).
    pub prev_state_root: Felt,

    /// The state root at the attested block.
    pub state_root: Felt,

    /// The hash of the previous block (or zero when not provided).
    pub prev_block_hash: BlockHash,

    /// The hash of the attested block.
    pub block_hash: BlockHash,

    /// The number of the previous block (or u64::MAX when not provided).
    pub prev_block_number: BlockNumber,

    /// The number of the attested block.
    pub block_number: BlockNumber,

    /// The block number Katana forked from (if running in fork mode).
    /// Attested by TEE hardware via report_data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_block_number: Option<BlockNumber>,

    /// Merkle root of all events in the attested block.
    /// Included in report_data: Poseidon(state_root, block_hash, fork_block, events_commitment).
    pub events_commitment: Felt,
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
