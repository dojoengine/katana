#![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[cfg(not(feature = "debug"))]
use cairo_lang_starknet_classes as _;

pub mod cli;
