use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

pub type EventFilterWithPage = starknet::core::types::EventFilterWithPage;

/// A "page" of events in a cursor-based pagniation system.
///
/// This type is usually returned from the `starknet_getEvents` RPC method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEventsResponse {
    /// Matching events
    pub events: Vec<EmittedEvent>,

    /// A pointer to the last element of the delivered page, use this token in a subsequent query
    /// to obtain the next page. If the value is `None`, don't add it to the response as
    /// clients might use `contains_key` as a check for the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

/// Emitted event.
///
/// Event information decorated with metadata on where it was emitted / an event emitted as a result
/// of transaction execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmittedEvent {
    pub from_address: ContractAddress,
    pub keys: Vec<Felt>,
    pub data: Vec<Felt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<BlockHash>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<BlockNumber>,
    pub transaction_hash: TxHash,
}
