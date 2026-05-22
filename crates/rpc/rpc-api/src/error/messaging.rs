use jsonrpsee::types::ErrorObjectOwned;

/// Error codes for the `messaging` namespace. Start at 200 to avoid collision
/// with other Katana RPC error enums.
#[derive(thiserror::Error, Clone, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum MessagingApiError {
    #[error("Messaging checkpoint storage error: {0}")]
    StorageError(String),
    #[error("Failed to signal messaging server for rewind: {0}")]
    RewindSignalFailed(String),
}

impl MessagingApiError {
    fn code(&self) -> i32 {
        match self {
            Self::StorageError(_) => 200,
            Self::RewindSignalFailed(_) => 201,
        }
    }
}

impl From<MessagingApiError> for ErrorObjectOwned {
    fn from(err: MessagingApiError) -> Self {
        ErrorObjectOwned::owned(err.code(), err.to_string(), None::<()>)
    }
}
