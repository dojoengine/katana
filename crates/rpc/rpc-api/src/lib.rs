#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod dev;
pub mod error;
pub mod starknet;
pub mod starknet_ext;

#[cfg(feature = "cartridge")]
pub mod cartridge;
#[cfg(feature = "cartridge")]
pub mod paymaster;
