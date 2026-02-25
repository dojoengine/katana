mod common;
pub mod types;

pub use common::*;

/// Starknet service definitions from starknet.proto
pub mod starknet {
    tonic::include_proto!("starknet");

    /// File descriptor set for gRPC reflection support.
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("starknet_descriptor");
}
