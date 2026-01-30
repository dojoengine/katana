//! gRPC server implementation for Katana.
//!
//! This crate provides gRPC endpoints for interacting with the Katana Starknet sequencer,
//! offering high-performance alternatives to JSON-RPC for performance-critical applications.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod config;
mod error;
mod handlers;
mod protos;
mod server;

// Re-export conversion module for internal use
pub use config::GrpcConfig;
pub use handlers::StarknetService;
pub use protos::starknet::starknet_server::StarknetServer;
pub use protos::starknet::starknet_trace_server::StarknetTraceServer;
pub use protos::starknet::starknet_write_server::StarknetWriteServer;
pub(crate) use protos::types::conversion;
pub use server::{GrpcServer, GrpcServerHandle};
