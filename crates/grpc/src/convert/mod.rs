//! Type conversion utilities between proto types and internal Katana types.
//!
//! This module provides bidirectional conversion between:
//! - Proto types generated from .proto files
//! - Internal Katana types from katana-primitives and katana-rpc-types
//!
//! Conversions are implemented using `From` and `TryFrom` traits for idiomatic Rust.

mod block;
mod felt;
mod receipt;
mod state;
mod trace;
mod transaction;

pub use block::*;
pub use felt::{FeltVecExt, ProtoFeltVecExt};
pub use receipt::*;
pub use state::*;
pub use trace::*;
pub use transaction::*;
