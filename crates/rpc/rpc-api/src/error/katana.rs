use jsonrpsee::types::ErrorObjectOwned;

#[derive(thiserror::Error, Clone, Copy, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum KatanaApiError {
    #[error("Failed to change next block timestamp.")]
    FailedToChangeNextBlockTimestamp = 1,
    #[error("Failed to dump state.")]
    FailedToDumpState = 2,
    #[error("Failed to update storage.")]
    FailedToUpdateStorage = 3,
}

impl From<KatanaApiError> for ErrorObjectOwned {
    fn from(err: KatanaApiError) -> Self {
        ErrorObjectOwned::owned(err as i32, err.to_string(), None::<()>)
    }
}
