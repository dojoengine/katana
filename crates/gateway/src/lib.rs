#![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[cfg(all(not(test), feature = "server"))]
use tokio as _;
#[cfg(not(test))]
use {anyhow as _, clap as _, serde_json as _, tracing as _};

pub mod client;
#[cfg(feature = "server")]
pub mod server;
pub mod types;
