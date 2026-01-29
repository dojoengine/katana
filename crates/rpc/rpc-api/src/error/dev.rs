use jsonrpsee::types::ErrorObjectOwned;
use serde::{Deserialize, Serialize};

#[derive(thiserror::Error, Clone, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum DevApiError {
    #[error("Wait for pending transactions.")]
    PendingTransactions,
    #[error("An unexpected error occurred: {}", .0.reason)]
    UnexpectedError(UnexpectedErrorData),
}

impl DevApiError {
    pub fn unexpected_error<T: ToString>(reason: T) -> Self {
        DevApiError::UnexpectedError(UnexpectedErrorData { reason: reason.to_string() })
    }
}

/// Data for the [`DevApiError::UnexpectedError`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnexpectedErrorData {
    pub reason: String,
}

impl From<DevApiError> for ErrorObjectOwned {
    fn from(err: DevApiError) -> Self {
        match &err {
            DevApiError::PendingTransactions => {
                ErrorObjectOwned::owned(1, err.to_string(), None::<()>)
            }
            DevApiError::UnexpectedError(data) => {
                ErrorObjectOwned::owned(2, err.to_string(), Some(data))
            }
        }
    }
}
