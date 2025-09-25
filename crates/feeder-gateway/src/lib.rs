#![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[cfg(not(test))]
use {anyhow as _, clap as _, serde_json as _, tokio as _};

pub mod client;
pub mod server;
pub mod types;
