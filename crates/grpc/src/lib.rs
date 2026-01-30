//! gRPC server implementation for Katana.
//!
//! This crate provides gRPC endpoints for interacting with the Katana Starknet sequencer,
//! offering high-performance alternatives to JSON-RPC for performance-critical applications.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

/// Generated protobuf types and service definitions.
pub mod protos {
    /// Types definitions from types.proto
    pub mod types {
        tonic::include_proto!("types");
    }

    /// Starknet service definitions from starknet.proto
    pub mod starknet {
        tonic::include_proto!("starknet");

        /// File descriptor set for gRPC reflection support.
        pub const FILE_DESCRIPTOR_SET: &[u8] =
            tonic::include_file_descriptor_set!("starknet_descriptor");
    }
}

mod config;
mod convert;
mod error;
mod handlers;
mod impls;
mod server;

pub use config::GrpcConfig;
pub use server::{GrpcServer, GrpcServerHandle};
