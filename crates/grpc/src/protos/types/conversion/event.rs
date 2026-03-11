//! Event type conversions.

use katana_primitives::block::BlockIdOrTag;
use katana_primitives::Felt;
use katana_rpc_types::event::{
    EmittedEvent, EventFilter, EventFilterWithPage, GetEventsResponse, ResultPageRequest,
};
use tonic::Status;

use super::FeltVecExt;
use crate::protos::common::Felt as ProtoFelt;
use crate::protos::starknet::{GetEventsRequest, GetEventsResponse as ProtoGetEventsResponse};
use crate::protos::types::EmittedEvent as ProtoEmittedEvent;

/// Convert GetEventsRequest to EventFilterWithPage
impl TryFrom<GetEventsRequest> for EventFilterWithPage {
    type Error = Status;

    fn try_from(req: GetEventsRequest) -> Result<Self, Self::Error> {
        let filter = req.filter.ok_or_else(|| Status::invalid_argument("Missing filter"))?;

        let from_block = filter.from_block.map(BlockIdOrTag::try_from).transpose()?;

        let to_block = filter.to_block.map(BlockIdOrTag::try_from).transpose()?;

        let address = filter.address.map(|f| f.try_into()).transpose()?;

        // Convert flat keys to nested structure (each key at position i matches exactly)
        let keys = if filter.keys.is_empty() {
            None
        } else {
            Some(
                filter.keys.into_iter().map(|k| Ok(vec![Felt::try_from(k)?])).collect::<Result<
                    Vec<Vec<Felt>>,
                    Status,
                >>(
                )?,
            )
        };

        let continuation_token =
            if req.continuation_token.is_empty() { None } else { Some(req.continuation_token) };

        Ok(EventFilterWithPage {
            event_filter: EventFilter { from_block, to_block, address, keys },
            result_page_request: ResultPageRequest {
                continuation_token,
                chunk_size: req.chunk_size as u64,
            },
        })
    }
}

/// Convert GetEventsResponse to proto
impl From<GetEventsResponse> for ProtoGetEventsResponse {
    fn from(response: GetEventsResponse) -> Self {
        ProtoGetEventsResponse {
            events: response.events.into_iter().map(ProtoEmittedEvent::from).collect(),
            continuation_token: response.continuation_token.unwrap_or_default(),
        }
    }
}

/// Convert EmittedEvent to proto
impl From<EmittedEvent> for ProtoEmittedEvent {
    fn from(event: EmittedEvent) -> Self {
        ProtoEmittedEvent {
            from_address: Some(ProtoFelt::from(Felt::from(event.from_address))),
            keys: event.keys.to_proto_felts(),
            data: event.data.to_proto_felts(),
            block_hash: event.block_hash.map(ProtoFelt::from),
            block_number: event.block_number.unwrap_or(0),
            transaction_hash: Some(event.transaction_hash.into()),
        }
    }
}
