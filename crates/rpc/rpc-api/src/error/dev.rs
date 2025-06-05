use jsonrpsee::types::ErrorObjectOwned;

#[derive(thiserror::Error, Clone, Copy, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum DevApiError {
    #[error("Wait for pending transactions.")]
    PendingTransactions,
}

impl From<DevApiError> for ErrorObjectOwned {
    fn from(err: DevApiError) -> Self {
        ErrorObjectOwned::owned(err as i32, err.to_string(), None::<()>)
    }
}
