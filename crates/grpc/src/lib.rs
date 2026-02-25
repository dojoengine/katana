//! gRPC server and client implementation for Katana.
//!
//! This crate provides gRPC endpoints for interacting with the Katana Starknet sequencer,
//! offering high-performance alternatives to JSON-RPC for performance-critical applications.
//!
//! # Server
//!
//! Use [`GrpcServer`] to create and start a gRPC server:
//!
//! ```ignore
//! use katana_grpc::{GrpcServer, StarknetServer, StarknetService};
//!
//! let service = StarknetService::new(/* ... */);
//! let server = GrpcServer::new()
//!     .service(StarknetServer::new(service))
//!     .start("127.0.0.1:5051".parse()?)
//!     .await?;
//! ```
//!
//! # Client
//!
//! Use [`GrpcClient`] to connect to a gRPC server:
//!
//! ```ignore
//! use katana_grpc::{GrpcClient, proto::ChainIdRequest};
//!
//! // Simple connection with defaults
//! let mut client = GrpcClient::connect("http://localhost:5051").await?;
//!
//! // Or use the builder for custom configuration
//! let mut client = GrpcClient::builder("http://localhost:5051")
//!     .timeout(Duration::from_secs(30))
//!     .connect()
//!     .await?;
//!
//! let response = client.chain_id(ChainIdRequest {}).await?;
//! println!("Chain ID: {}", response.into_inner().chain_id);
//! ```

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod client;
mod error;
mod handlers;
mod protos;
mod server;

// Client exports
pub use client::{Error as ClientError, GrpcClient, GrpcClientBuilder};
// Server exports
pub use handlers::StarknetService;
pub use protos::starknet::starknet_server::StarknetServer;
pub use protos::starknet::starknet_trace_server::StarknetTraceServer;
pub use protos::starknet::starknet_write_server::StarknetWriteServer;
pub use server::{GrpcServer, GrpcServerHandle};

/// Protocol buffer generated types.
///
/// This module contains all the request/response types needed to interact with
/// the gRPC services.
pub mod proto {
    pub use super::protos::starknet::*;
    pub use super::protos::types::*;
    pub use super::protos::*;
}

// Re-export conversion module for internal use
pub(crate) use protos::types::conversion;
