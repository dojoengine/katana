use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::cairo::ShortString;
use katana_primitives::hash::{Pedersen, StarkHash};
use katana_primitives::Felt;
use katana_rpc_types::trie::Nodes;
use serde::{Deserialize, Serialize};

/// Version tag for the Katana TEE environment config hash.
pub const KATANA_TEE_CONFIG_VERSION: ShortString = ShortString::from_ascii("KatanaTeeConfig1");

/// Version tag for the v1 Katana TEE report-data schema.
pub const KATANA_TEE_REPORT_VERSION: ShortString = ShortString::from_ascii("KatanaTeeReport1");

/// Mode tag for appchain settlement attestations.
pub const KATANA_TEE_APPCHAIN_MODE: ShortString = ShortString::from_ascii("KatanaTeeAppchain");

/// Mode tag for fork/sharding attestations.
pub const KATANA_TEE_SHARDING_MODE: ShortString = ShortString::from_ascii("KatanaTeeSharding");

/// Computes the Starknet-OS-style config hash bound into v1 TEE report data.
///
/// This mirrors the Starknet OS pattern of hashing a version tag plus stable
/// environment fields. The non-legacy fee token address is the STRK fee token
/// address for the appchain/Katana network.
pub fn compute_katana_tee_config_hash(
    appchain_chain_id_or_katana_chain_id: Felt,
    fee_token_address: Felt,
) -> Felt {
    Pedersen::hash_array(&[
        KATANA_TEE_CONFIG_VERSION.into(),
        appchain_chain_id_or_katana_chain_id,
        fee_token_address,
    ])
}

/// Serializes an `Option<BlockNumber>` as a `Felt`-encoded hex string, mapping
/// `None` to `Felt::MAX` (the genesis "no previous block" sentinel that
/// matches Piltover's `AppchainState` initial value). Deserializes the inverse:
/// `Felt::MAX` becomes `None`, anything else becomes `Some(felt as u64)`.
///
/// The wire format is `Felt`, not JSON `null`, so the field can be hashed into
/// the on-chain commitment without a separate sentinel encoding step.
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

    /// The number of the previous block. Serialized as a `Felt`-encoded hex
    /// string (with `Felt::MAX` representing the genesis "no previous block"
    /// case) so the field can be hashed into the on-chain commitment without
    /// a separate sentinel encoding step. See [`prev_block_number_serde`].
    #[serde(with = "prev_block_number_serde")]
    pub prev_block_number: Option<BlockNumber>,

    /// The number of the attested block. Serialized as a `0x`-prefixed hex
    /// string matching `Felt`'s default serde representation, so the field
    /// can be hashed as a `Felt` on both sides of the wire.
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
    /// Included in fork/sharding report_data.
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
    /// The quote commits to the block's transition fields and to a versioned
    /// `katana_tee_config_hash` that the node precomputes from its chain spec
    /// at startup. Verifiers can recompute the same hash from on-chain config
    /// and reject attestations bound to a different environment.
    ///
    /// `prev_block_id` is optional and included in the response for
    /// transition-style flows; `None` is the genesis case.
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
