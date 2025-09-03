#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use hex as _;

pub mod block;
pub mod cairo;
pub mod chain;
pub mod class;
pub mod contract;
pub mod da;
pub mod env;
pub mod eth;
pub mod event;
pub mod execution;
pub mod fee;
pub mod genesis;
pub mod message;
pub mod receipt;
pub mod transaction;
pub mod version;

pub mod state;
pub mod utils;

mod macros;

pub use alloy_primitives::{B256, U256};
pub use contract::ContractAddress;
pub use starknet::macros::felt;
pub use starknet_types_core::felt::{Felt, FromStrError};
pub use starknet_types_core::hash;

pub mod _private {
    pub use ::alloy_primitives;
}
