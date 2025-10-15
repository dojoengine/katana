use std::fmt;

use katana_rpc_api::error::starknet::StarknetApiError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, thiserror::Error)]
#[error("{message} ({code})")]
pub struct GatewayError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default)]
    pub problems: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    BlockNotFound,
    EntryPointNotFoundInContract,
    InvalidProgram,
    TransactionFailed,
    TransactionNotFound,
    UninitializedContract,
    MalformedRequest,
    UndeclaredClass,
    InvalidTransactionNonce,
    ValidateFailure,
    ClassAlreadyDeclared,
    CompilationFailed,
    InvalidCompiledClassHash,
    DuplicatedTransaction,
    InvalidContractClass,
    DeprecatedEndpoint,
    NotFound,
    Unknown(String),
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
            StarknetApiError::CompilationError(_) => ErrorCode::CompilationFailed,
            StarknetApiError::CompiledClassHashMismatch => ErrorCode::InvalidCompiledClassHash,
            StarknetApiError::DuplicateTransaction => ErrorCode::DuplicatedTransaction,
            StarknetApiError::InvalidContractClass => ErrorCode::InvalidContractClass,
            StarknetApiError::TransactionExecutionError(_) => ErrorCode::TransactionFailed,
            StarknetApiError::ContractNotFound => ErrorCode::UninitializedContract,
            StarknetApiError::InvalidCallData => ErrorCode::MalformedRequest,
            _ => return Err(value), // Return back the error for unmapped variants
        };

        Ok(GatewayError { code, message: value.to_string(), problems: None })
    }
}

impl ErrorCode {
    fn as_str(&self) -> &str {
        match self {
            ErrorCode::NotFound => "404",
            ErrorCode::BlockNotFound => "StarknetErrorCode.BLOCK_NOT_FOUND",
            ErrorCode::EntryPointNotFoundInContract => {
                "StarknetErrorCode.ENTRY_POINT_NOT_FOUND_IN_CONTRACT"
            }
            ErrorCode::InvalidProgram => "StarknetErrorCode.INVALID_PROGRAM",
            ErrorCode::TransactionFailed => "StarknetErrorCode.TRANSACTION_FAILED",
            ErrorCode::TransactionNotFound => "StarknetErrorCode.TRANSACTION_NOT_FOUND",
            ErrorCode::UninitializedContract => "StarknetErrorCode.UNINITIALIZED_CONTRACT",
            ErrorCode::MalformedRequest => "StarkErrorCode.MALFORMED_REQUEST",
            ErrorCode::UndeclaredClass => "StarknetErrorCode.UNDECLARED_CLASS",
            ErrorCode::InvalidTransactionNonce => "StarknetErrorCode.INVALID_TRANSACTION_NONCE",
            ErrorCode::ValidateFailure => "StarknetErrorCode.VALIDATE_FAILURE",
            ErrorCode::ClassAlreadyDeclared => "StarknetErrorCode.CLASS_ALREADY_DECLARED",
            ErrorCode::CompilationFailed => "StarknetErrorCode.COMPILATION_FAILED",
            ErrorCode::InvalidCompiledClassHash => "StarknetErrorCode.INVALID_COMPILED_CLASS_HASH",
            ErrorCode::DuplicatedTransaction => "StarknetErrorCode.DUPLICATED_TRANSACTION",
            ErrorCode::InvalidContractClass => "StarknetErrorCode.INVALID_CONTRACT_CLASS",
            ErrorCode::DeprecatedEndpoint => "StarknetErrorCode.DEPRECATED_ENDPOINT",
            ErrorCode::Unknown(code) => code.as_str(),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let code = match value.as_str() {
            "404" => ErrorCode::NotFound,
            "StarknetErrorCode.BLOCK_NOT_FOUND" => ErrorCode::BlockNotFound,
            "StarknetErrorCode.ENTRY_POINT_NOT_FOUND_IN_CONTRACT" => {
                ErrorCode::EntryPointNotFoundInContract
            }
            "StarknetErrorCode.INVALID_PROGRAM" => ErrorCode::InvalidProgram,
            "StarknetErrorCode.TRANSACTION_FAILED" => ErrorCode::TransactionFailed,
            "StarknetErrorCode.TRANSACTION_NOT_FOUND" => ErrorCode::TransactionNotFound,
            "StarknetErrorCode.UNINITIALIZED_CONTRACT" => ErrorCode::UninitializedContract,
            "StarkErrorCode.MALFORMED_REQUEST" => ErrorCode::MalformedRequest,
            "StarknetErrorCode.UNDECLARED_CLASS" => ErrorCode::UndeclaredClass,
            "StarknetErrorCode.INVALID_TRANSACTION_NONCE" => ErrorCode::InvalidTransactionNonce,
            "StarknetErrorCode.VALIDATE_FAILURE" => ErrorCode::ValidateFailure,
            "StarknetErrorCode.CLASS_ALREADY_DECLARED" => ErrorCode::ClassAlreadyDeclared,
            "StarknetErrorCode.COMPILATION_FAILED" => ErrorCode::CompilationFailed,
            "StarknetErrorCode.INVALID_COMPILED_CLASS_HASH" => ErrorCode::InvalidCompiledClassHash,
            "StarknetErrorCode.DUPLICATED_TRANSACTION" => ErrorCode::DuplicatedTransaction,
            "StarknetErrorCode.INVALID_CONTRACT_CLASS" => ErrorCode::InvalidContractClass,
            "StarknetErrorCode.DEPRECATED_ENDPOINT" => ErrorCode::DeprecatedEndpoint,
            other => ErrorCode::Unknown(other.to_string()),
        };

        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ErrorCode, GatewayError};

    #[test]
    fn not_found_error() {
        let json = json!({
          "code": "404",
          "message": "404: Not Found",
          "problems": "Not Found"
        });

        let error = serde_json::from_value::<GatewayError>(json).unwrap();
        assert!(matches!(error, GatewayError { code: ErrorCode::NotFound, .. }));
    }
}
