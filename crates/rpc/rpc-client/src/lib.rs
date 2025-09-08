#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};

pub mod starknet;
