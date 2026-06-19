use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

/// Events request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFilterWithPage {
    #[serde(flatten)]
    pub event_filter: EventFilter,
    #[serde(flatten)]
    pub result_page_request: ResultPageRequest,
}

/// Event filter.
///
/// An event filter/query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    ///
    /// Per key (by position), designate the possible values to be matched for events to be
    /// returned. Empty array designates 'any' value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<Vec<Felt>>>,
}

/// Result page request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The hash of the block in which the event was emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<BlockHash>,
    /// The number of the block in which the event was emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<BlockNumber>,
    /// The hash of the transaction where the event was emitted.
    pub transaction_hash: TxHash,
    /// The index of the transaction in the block.
    #[serde(default)]
    pub transaction_index: u64,
    /// The index of the event within the transaction.
    #[serde(default)]
    pub event_index: u64,
    /// The address of the contract that emitted the event.
    pub from_address: ContractAddress,

    /// The event's `keys` and `data`. Flattened to the top level so the JSON
    /// matches the Starknet RPC `EMITTED_EVENT` shape (spec-compliant clients and
    /// indexers like Torii read `keys`/`data` directly, not nested under `event`).
    #[serde(flatten)]
    pub event: RawEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawEvent {
    pub keys: Vec<Felt>,
    pub data: Vec<Felt>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EmittedEvent` must serialize `keys`/`data` at the top level (the Starknet
    /// RPC `EMITTED_EVENT` shape), not nested under an `event` object — otherwise
    /// spec-compliant clients/indexers (e.g. Torii) fail to deserialize the
    /// `getEvents` response with "missing field `keys`".
    #[test]
    fn emitted_event_serializes_keys_and_data_flat() {
        let event = EmittedEvent {
            block_hash: Some(Felt::from(1u8)),
            block_number: Some(2),
            transaction_hash: Felt::from(3u8),
            transaction_index: 0,
            event_index: 0,
            from_address: ContractAddress::from(Felt::from(4u8)),
            event: RawEvent { keys: vec![Felt::from(5u8)], data: vec![Felt::from(6u8)] },
        };

        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("keys").is_some(), "`keys` must be a top-level field");
        assert!(json.get("data").is_some(), "`data` must be a top-level field");
        assert!(json.get("event").is_none(), "`keys`/`data` must not be nested under `event`");

        // And it round-trips from the flat shape.
        assert_eq!(serde_json::from_value::<EmittedEvent>(json).unwrap(), event);
    }
}
