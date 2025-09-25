use katana_rpc_api::error::starknet::StarknetApiError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, thiserror::Error)]
#[error("{message} ({code:?})")]
pub struct GatewayError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ErrorCode {
    #[serde(rename = "StarknetErrorCode.BLOCK_NOT_FOUND")]
    BlockNotFound,
    #[serde(rename = "StarknetErrorCode.ENTRY_POINT_NOT_FOUND_IN_CONTRACT")]
    EntryPointNotFoundInContract,
    #[serde(rename = "StarknetErrorCode.INVALID_PROGRAM")]
    InvalidProgram,
    #[serde(rename = "StarknetErrorCode.TRANSACTION_FAILED")]
    TransactionFailed,
    #[serde(rename = "StarknetErrorCode.TRANSACTION_NOT_FOUND")]
    TransactionNotFound,
    #[serde(rename = "StarknetErrorCode.UNINITIALIZED_CONTRACT")]
    UninitializedContract,
    #[serde(rename = "StarkErrorCode.MALFORMED_REQUEST")]
    MalformedRequest,
    #[serde(rename = "StarknetErrorCode.UNDECLARED_CLASS")]
    UndeclaredClass,
    #[serde(rename = "StarknetErrorCode.INVALID_TRANSACTION_NONCE")]
    InvalidTransactionNonce,
    #[serde(rename = "StarknetErrorCode.VALIDATE_FAILURE")]
    ValidateFailure,
    #[serde(rename = "StarknetErrorCode.CLASS_ALREADY_DECLARED")]
    ClassAlreadyDeclared,
    #[serde(rename = "StarknetErrorCode.COMPILATION_FAILED")]
    CompilationFailed,
    #[serde(rename = "StarknetErrorCode.INVALID_COMPILED_CLASS_HASH")]
    InvalidCompiledClassHash,
    #[serde(rename = "StarknetErrorCode.DUPLICATED_TRANSACTION")]
    DuplicatedTransaction,
    #[serde(rename = "StarknetErrorCode.INVALID_CONTRACT_CLASS")]
    InvalidContractClass,
    #[serde(rename = "StarknetErrorCode.DEPRECATED_ENDPOINT")]
    DeprecatedEndpoint,
}

impl TryFrom<StarknetApiError> for GatewayError {
    type Error = StarknetApiError;

    fn try_from(value: StarknetApiError) -> Result<Self, Self::Error> {
        let code = match &value {
            StarknetApiError::BlockNotFound => ErrorCode::BlockNotFound,
            StarknetApiError::EntrypointNotFound => ErrorCode::EntryPointNotFoundInContract,
            StarknetApiError::TxnHashNotFound => ErrorCode::TransactionNotFound,
            StarknetApiError::ClassHashNotFound => ErrorCode::UndeclaredClass,
            StarknetApiError::InvalidTransactionNonce(_) => ErrorCode::InvalidTransactionNonce,
            StarknetApiError::ValidationFailure(_) => ErrorCode::ValidateFailure,
            StarknetApiError::ClassAlreadyDeclared => ErrorCode::ClassAlreadyDeclared,
            StarknetApiError::CompilationFailed(_) => ErrorCode::CompilationFailed,
            StarknetApiError::CompiledClassHashMismatch => ErrorCode::InvalidCompiledClassHash,
            StarknetApiError::DuplicateTransaction => ErrorCode::DuplicatedTransaction,
            StarknetApiError::InvalidContractClass => ErrorCode::InvalidContractClass,
            StarknetApiError::TransactionExecutionError(_) => ErrorCode::TransactionFailed,
            StarknetApiError::ContractNotFound => ErrorCode::UninitializedContract,
            StarknetApiError::InvalidCallData => ErrorCode::MalformedRequest,
            _ => return Err(value), // Return back the error for unmapped variants
        };

        Ok(GatewayError { code, message: value.to_string() })
    }
}
