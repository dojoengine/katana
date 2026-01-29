use jsonrpsee::types::ErrorObjectOwned;

/// Error codes for TEE API (starting at 100 to avoid conflicts).
const TEE_NOT_AVAILABLE: i32 = 100;
const TEE_QUOTE_GENERATION_FAILED: i32 = 101;
const TEE_PROVIDER_ERROR: i32 = 102;

#[derive(thiserror::Error, Clone, Debug)]
pub enum TeeApiError {
    #[error("TEE not available: {0}")]
    NotAvailable(String),

    #[error("Quote generation failed: {0}")]
    QuoteGenerationFailed(String),

    #[error("Provider error: {0}")]
    ProviderError(String),
}

impl From<TeeApiError> for ErrorObjectOwned {
    fn from(err: TeeApiError) -> Self {
        let code = match &err {
            TeeApiError::NotAvailable(_) => TEE_NOT_AVAILABLE,
            TeeApiError::QuoteGenerationFailed(_) => TEE_QUOTE_GENERATION_FAILED,
            TeeApiError::ProviderError(_) => TEE_PROVIDER_ERROR,
        };
        ErrorObjectOwned::owned(code, err.to_string(), None::<()>)
    }
}
