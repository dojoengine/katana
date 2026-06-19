use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

use crate::trie::Nodes;
use crate::{L1ToL2Message, L2ToL1Message};

/// Response type for TEE quote generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockAttestation {
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

    /// The number of the previous block. Serialized as a `Felt`-encoded hex
    /// string (with `Felt::MAX` representing the genesis "no previous block"
    /// case) so the JSON wire format matches what
    /// `katana_tee_client::TeeQuoteResponse` (used by `saya-tee`) expects.
    /// See [`prev_block_number_serde`].
    pub prev_block_number: Felt,

    /// The number of the attested block. Serialized as a `0x`-prefixed hex
    /// string so the JSON wire format matches `Felt`'s default serde
    /// representation, which is what `katana_tee_client::TeeQuoteResponse`
    /// (typed as `Felt` upstream) expects.
    pub block_number: Felt,

    /// The block number Katana forked from (if running in fork mode).
    /// Attested by TEE hardware via report_data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_block_number: Option<Felt>,

    /// Merkle root of all events in the attested block.
    /// Included in report_data: Poseidon(state_root, block_hash, fork_block, events_commitment).
    pub events_commitment: Felt,

    /// Poseidon commitment over all L1<->L2 messages from prev_block+1 to block_number.
    ///
    /// Computed as `Poseidon(l2_to_l1_commitment, l1_to_l2_commitment)` where each direction's
    /// commitment is `Poseidon` over the individual message hashes in that range.
    pub messages_commitment: Felt,

    /// Versioned Katana TEE environment config hash attested in `report_data`.
    ///
    /// Always non-zero in v1 — precomputed by the node from its chain spec.
    pub katana_tee_config_hash: Felt,

    /// All L2→L1 messages emitted in the attested block range.
    pub l2_to_l1_messages: Vec<L2ToL1Message>,

    /// All L1→L2 messages processed in the attested block range.
    pub l1_to_l2_messages: Vec<L1ToL2Message>,
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
