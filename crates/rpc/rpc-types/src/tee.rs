use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

use crate::trie::Nodes;
use crate::{L1ToL2Message, L2ToL1Message};

// /// A L1→L2 message derived from an L1Handler transaction.
// ///
// /// All fields are required to independently recompute the `message_hash`:
// /// `keccak256(from_address_u256, to_address, nonce, selector, payload.len, payload...)`.
// #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct TeeL1ToL2Message {
//     /// Ethereum address of the L1 sender (padded to felt).
//     pub from_address: Felt,
//     /// L2 contract address (the L1Handler target).
//     pub to_address: Felt,
//     /// Entry point selector of the L1Handler function.
//     pub selector: Felt,
//     /// Message payload (excludes the prepended from_address in calldata).
//     pub payload: Vec<Felt>,
//     /// Message nonce assigned by the core contract on L1.
//     pub nonce: Felt,
// }

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
    #[serde(with = "prev_block_number_serde")]
    pub prev_block_number: Option<BlockNumber>,

    /// The number of the attested block. Serialized as a `0x`-prefixed hex
    /// string so the JSON wire format matches `Felt`'s default serde
    /// representation, which is what `katana_tee_client::TeeQuoteResponse`
    /// (typed as `Felt` upstream) expects.
    #[serde(
        serialize_with = "serde_utils::serialize_as_hex",
        deserialize_with = "serde_utils::deserialize_u64"
    )]
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

/// Serializes an `Option<BlockNumber>` as a `Felt`-encoded hex string, mapping
/// `None` to `Felt::MAX` (the genesis "no previous block" sentinel that
/// matches Piltover's `AppchainState` initial value). Deserializes the inverse:
/// `Felt::MAX` becomes `None`, anything else becomes `Some(felt as u64)`.
///
/// The non-optional `block_number` field uses `serde_utils::serialize_as_hex` +
/// `serde_utils::deserialize_u64` directly. We can't reuse `serde_utils::{serialize_opt_as_hex,
/// deserialize_opt_u64}` here because those map `None ↔ null`, not `None ↔ Felt::MAX`.
mod prev_block_number_serde {
    use katana_primitives::block::BlockNumber;
    use katana_primitives::Felt;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(n: &Option<BlockNumber>, s: S) -> Result<S::Ok, S::Error> {
        let felt = match n {
            Some(n) => Felt::from(*n),
            None => Felt::MAX,
        };
        felt.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<BlockNumber>, D::Error> {
        let felt = Felt::deserialize(d)?;
        if felt == Felt::MAX {
            Ok(None)
        } else {
            u64::try_from(felt).map(Some).map_err(serde::de::Error::custom)
        }
    }
}
