use jsonrpsee::types::ErrorObjectOwned;
use katana_pool_api::PoolError;
use katana_provider_api::ProviderError;

/// Error codes for Cartridge API (starting at 200 to avoid conflicts).
const CONTROLLER_DEPLOYMENT_FAILED: i32 = 200;
const VRF_MISSING_FOLLOW_UP_CALL: i32 = 201;
const VRF_INVALID_TARGET: i32 = 202;
const VRF_EXECUTION_FAILED: i32 = 203;
const PAYMASTER_EXECUTION_FAILED: i32 = 204;
const POOL_ERROR: i32 = 205;
const PROVIDER_ERROR: i32 = 206;
const INTERNAL_ERROR: i32 = 299;

#[derive(Debug, thiserror::Error, Clone)]
pub enum CartridgeApiError {
    /// Failed to deploy a Cartridge controller account.
    #[error("Controller deployment failed: {reason}")]
    ControllerDeployment { reason: String },

    /// The `request_random` call is not followed by another call in the outside execution.
    #[error("request_random call must be followed by another call")]
    VrfMissingFollowUpCall,

    /// The `request_random` call does not target the expected VRF account.
    #[error("request_random call must target the VRF account")]
    VrfInvalidTarget,

    /// The VRF outside execution request failed.
    ///
    /// Error returns by the VRF server.
    #[error("VRF execution failed: {reason}")]
    VrfExecutionFailed { reason: String },

    /// The paymaster failed to execute the transaction.
    ///
    /// Error returns by the Paymaster server.
    #[error("Paymaster execution failed: {reason}")]
    PaymasterExecutionFailed { reason: String },

    /// Failed to submit transaction to the pool.
    #[error("Transaction pool error: {reason}")]
    PoolError { reason: String },

    /// Storage provider error.
    #[error("Provider error: {reason}")]
    ProviderError { reason: String },

    /// Internal error (e.g., task execution failure).
    #[error("Internal error: {reason}")]
    InternalError { reason: String },
}

impl From<CartridgeApiError> for ErrorObjectOwned {
    fn from(err: CartridgeApiError) -> Self {
        let code = match &err {
            CartridgeApiError::ControllerDeployment { .. } => CONTROLLER_DEPLOYMENT_FAILED,
            CartridgeApiError::VrfMissingFollowUpCall => VRF_MISSING_FOLLOW_UP_CALL,
            CartridgeApiError::VrfInvalidTarget => VRF_INVALID_TARGET,
            CartridgeApiError::VrfExecutionFailed { .. } => VRF_EXECUTION_FAILED,
            CartridgeApiError::PaymasterExecutionFailed { .. } => PAYMASTER_EXECUTION_FAILED,
            CartridgeApiError::PoolError { .. } => POOL_ERROR,
            CartridgeApiError::ProviderError { .. } => PROVIDER_ERROR,
            CartridgeApiError::InternalError { .. } => INTERNAL_ERROR,
        };

        ErrorObjectOwned::owned(code, err.to_string(), None::<()>)
    }
}

impl From<ProviderError> for CartridgeApiError {
    fn from(value: ProviderError) -> Self {
        CartridgeApiError::ProviderError { reason: value.to_string() }
    }
}

impl From<anyhow::Error> for CartridgeApiError {
    fn from(value: anyhow::Error) -> Self {
        CartridgeApiError::ControllerDeployment { reason: value.to_string() }
    }
}

impl From<PoolError> for CartridgeApiError {
    fn from(error: PoolError) -> Self {
        CartridgeApiError::PoolError { reason: error.to_string() }
    }
}
