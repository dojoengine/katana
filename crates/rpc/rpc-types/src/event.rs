use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

/// Events request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventFilterWithPage {
    #[serde(flatten)]
    pub event_filter: EventFilter,
    #[serde(flatten)]
    pub result_page_request: ResultPageRequest,
}

/// Event filter.
///
/// An event filter/query.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventFilter {
    /// From block
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_block: Option<BlockIdOrTag>,
    /// To block
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_block: Option<BlockIdOrTag>,
    /// From contract
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<ContractAddress>,
    /// The keys to filter over
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<Vec<Felt>>>,
}

/// Result page request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResultPageRequest {
    /// The token returned from the previous query. If no token is provided the first page is
    /// returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
    /// Chunk size
    pub chunk_size: u64,
}

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
