//! Event types for Starknet spec v0.10.0.
//!
//! In v0.10, `event_index` and `transaction_index` are required fields (not optional).

use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

// Re-export unchanged types.
pub use crate::event::{EventFilter, EventFilterWithPage, ResultPageRequest};

/// A "page" of events in a cursor-based pagination system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEventsResponse {
    /// Matching events
    pub events: Vec<EmittedEvent>,

    /// A pointer to the last element of the delivered page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

/// Emitted event for v0.10 — `event_index` and `transaction_index` are required.
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
    /// The index of the transaction in the block (required in v0.10).
    pub transaction_index: u64,
    /// The index of the event within the transaction (required in v0.10).
    pub event_index: u64,
    /// The address of the contract that emitted the event.
    pub from_address: ContractAddress,
    pub keys: Vec<Felt>,
    pub data: Vec<Felt>,
}

impl From<crate::event::EmittedEvent> for EmittedEvent {
    fn from(e: crate::event::EmittedEvent) -> Self {
        Self {
            block_hash: e.block_hash,
            block_number: e.block_number,
            transaction_hash: e.transaction_hash,
            transaction_index: e.transaction_index.unwrap_or(0),
            event_index: e.event_index.unwrap_or(0),
            from_address: e.from_address,
            keys: e.keys,
            data: e.data,
        }
    }
}

impl From<crate::event::GetEventsResponse> for GetEventsResponse {
    fn from(r: crate::event::GetEventsResponse) -> Self {
        Self {
            events: r.events.into_iter().map(Into::into).collect(),
            continuation_token: r.continuation_token,
        }
    }
}
