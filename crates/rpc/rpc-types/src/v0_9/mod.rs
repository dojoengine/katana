//! Starknet JSON-RPC types for spec version 0.9.0.
//!
//! This module re-exports the existing types which are already v0.9-compatible.

pub mod block {
    pub use crate::block::{
        BlockHashAndNumberResponse, BlockNumberResponse, BlockTxCount, BlockWithReceipts,
        BlockWithTxHashes, BlockWithTxs, GetBlockWithReceiptsResponse,
        GetBlockWithTxHashesResponse, MaybePreConfirmedBlock, PreConfirmedBlockWithReceipts,
        PreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxs, RpcTxWithReceipt,
    };
}

pub mod event {
    pub use crate::event::{EmittedEvent, EventFilterWithPage, GetEventsResponse};
}

pub mod state_update {
    pub use crate::state_update::{
        ConfirmedStateUpdate, PreConfirmedStateUpdate, StateDiff, StateUpdate,
    };
}
