//! Type conversion utilities between proto types and internal Katana types.
//!
//! This module provides bidirectional conversion between:
//! - Proto types generated from .proto files
//! - Internal Katana types from katana-primitives and katana-rpc-types
//!
//! Conversions are implemented using `From` and `TryFrom` traits for idiomatic Rust.

mod block;
mod receipt;
mod state;
mod trace;
mod transaction;

pub use block::*;
use katana_primitives::Felt;
// These modules implement From/TryFrom traits for type conversions
// The types are used via .into() or TryFrom::try_from() calls
#[allow(unused_imports)]
pub use receipt::*;
#[allow(unused_imports)]
pub use state::*;
use tonic::Status;
#[allow(unused_imports)]
pub use trace::*;
#[allow(unused_imports)]
pub use transaction::*;

use crate::proto;

impl From<katana_primitives::Felt> for proto::Felt {
    fn from(felt: katana_primitives::Felt) -> Self {
        Self { value: felt.to_bytes_be().to_vec() }
    }
}

impl TryFrom<proto::Felt> for katana_primitives::Felt {
    type Error = Status;

    fn try_from(proto: proto::Felt) -> Result<Self, Self::Error> {
        if proto.value.len() > 32 {
            return Err(Status::invalid_argument("Felt value exceeds 32 bytes"));
        }

        Ok(Felt::from_bytes_be_slice(&proto.value))
    }
}

impl From<katana_primitives::ContractAddress> for proto::Felt {
    fn from(address: katana_primitives::ContractAddress) -> Self {
        Self { value: address.to_bytes_be().to_vec() }
    }
}

impl TryFrom<proto::Felt> for katana_primitives::ContractAddress {
    type Error = Status;

    fn try_from(value: proto::Felt) -> Result<Self, Self::Error> {
        Ok(Self::new(value.try_into()?))
    }
}

/// Extension trait for converting vectors of Felts.
pub trait FeltVecExt {
    fn to_proto_felts(&self) -> Vec<proto::Felt>;
}

impl FeltVecExt for [Felt] {
    fn to_proto_felts(&self) -> Vec<proto::Felt> {
        self.iter().copied().map(proto::Felt::from).collect()
    }
}
