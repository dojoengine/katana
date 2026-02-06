#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod dev;
pub mod error;
pub mod starknet;
pub mod starknet_ext;

#[cfg(feature = "cartridge")]
pub mod cartridge;

#[cfg(feature = "paymaster")]
pub mod paymaster {
    pub use katana_paymaster::api::*;
}

#[cfg(feature = "tee")]
pub mod tee;
