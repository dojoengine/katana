//! Starknet JSON-RPC types for spec version 0.10.0.
//!
//! This module defines types that differ from the v0.9 spec. The key changes are:
//!
//! - **BlockHeader**: Adds 7 commitment/count fields.
//! - **EmittedEvent**: `event_index` and `transaction_index` are required (not optional).
//! - **StateDiff**: `migrated_compiled_classes` is required (always serialized).
//! - **PreConfirmedStateUpdate**: `old_root` is optional.

pub mod block;
pub mod event;
pub mod state_update;
