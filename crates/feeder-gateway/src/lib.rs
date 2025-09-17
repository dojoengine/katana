#![cfg_attr(not(test), warn(unused_crate_dependencies))]
// Allow these deps as they're used by the feeder-cli binary
#![allow(unused_crate_dependencies)]

pub mod client;
pub mod types;
