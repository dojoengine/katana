//! gRPC server implementation for Katana.
//!
//! This crate provides gRPC endpoints for interacting with the Katana Starknet sequencer,
//! offering high-performance alternatives to JSON-RPC for performance-critical applications.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod config;
mod error;
mod handlers;
mod impls;
mod protos;
mod server;

pub use config::GrpcConfig;
pub use server::{GrpcServer, GrpcServerHandle};
