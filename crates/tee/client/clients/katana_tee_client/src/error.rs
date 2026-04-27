//! Error types for Katana TEE Client

use thiserror::Error;

/// Errors that can occur in the Katana TEE Client.
#[derive(Error, Debug)]
pub enum Error {
    /// Errors from the AMD TEE registry client
    #[error(transparent)]
    Registry(#[from] amd_tee_registry_client::Error),

    /// Katana RPC-specific errors
    #[error("Katana RPC error: {0}")]
    Rpc(String),
}
