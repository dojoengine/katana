//! gRPC service handlers.
//!
//! This module contains the implementations of the gRPC services defined in the proto files.
//! The handlers delegate to the underlying `StarknetApi` implementation for business logic.

mod starknet;

pub use self::starknet::StarknetService;
