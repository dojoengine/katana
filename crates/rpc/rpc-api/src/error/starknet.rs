use katana_primitives::block::BlockNumber;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Possible list of errors that can be returned by the Starknet API according to the spec: <https://github.com/starkware-libs/starknet-specs>.
#[derive(Debug, thiserror::Error, Clone, Serialize)]
#[serde(untagged)]
#[repr(i32)]
pub enum StarknetApiError {
    #[error("Failed to write transaction")]
    FailedToReceiveTxn,

    #[error("Contract not found")]
    ContractNotFound,

    #[error("Invalid call data")]
    InvalidCallData,

    #[error("Block not found")]
    BlockNotFound,

    #[error("Transaction hash not found")]
    TxnHashNotFound,

    #[error("Invalid transaction index in a block")]
    InvalidTxnIndex,

    #[error("Class hash not found")]
    ClassHashNotFound,

    #[error("Requested page size is too big")]
    PageSizeTooBig(PageSizeTooBigData),

    #[error("There are no blocks")]
    NoBlocks,

    #[error("The supplied continuation token is invalid or unknown")]
    InvalidContinuationToken,

    #[error("Contract error")]
    ContractError(ContractErrorData),

    #[error("Transaction execution error")]
    TransactionExecutionError(TransactionExecutionErrorData),

    #[error("Invalid contract class")]
    InvalidContractClass,

    #[error("Class already declared")]
    ClassAlreadyDeclared,

    // TEMP: adding a reason field temporarily to match what's being returned by the gateway. the
    // gateway includes the information regarding the expected and actual nonce in the error
    // message. but this doesn't break compatibility with the spec.
    #[error("Invalid transaction nonce")]
    InvalidTransactionNonce(InvalidTransactionNonceData),

    #[error("Account balance is smaller than the transaction's max_fee")]
    InsufficientAccountBalance,

    #[error("Account validation failed")]
    ValidationFailure(ValidationFailureData),

    #[error("Compilation failed")]
    CompilationFailed(CompilationFailedData),

    #[error("Contract class size is too large")]
    ContractClassSizeIsTooLarge,

    #[error("Sender address in not an account contract")]
    NonAccount,

    #[error("A transaction with the same hash already exists in the mempool")]
    DuplicateTransaction,

    #[error("The compiled class hash did not match the one supplied in the transaction")]
    CompiledClassHashMismatch,

    #[error("The transaction version is not supported")]
    UnsupportedTransactionVersion,

    #[error("The contract class version is not supported")]
    UnsupportedContractClassVersion,

    #[error("An unexpected error occurred")]
    UnexpectedError(UnexpectedErrorData),

    #[error("Too many keys provided in a filter")]
    TooManyKeysInFilter,

    #[error("Failed to fetch pending transactions")]
    FailedToFetchPendingTransactions,

    #[error("The node doesn't support storage proofs for blocks that are too far in the past")]
    StorageProofNotSupported(StorageProofNotSupportedData),

    #[error("Proof limit exceeded")]
    ProofLimitExceeded(ProofLimitExceededData),

    #[error("Requested entrypoint does not exist in the contract")]
    EntrypointNotFound,

    #[error("The transaction's resources don't cover validation or the minimal transaction fee")]
    InsufficientResourcesForValidate,

    #[error("Invalid subscription id")]
    InvalidSubscriptionId,

    #[error("Too many addresses in filter sender_address filter")]
    TooManyAddressesInFilter,

    #[error("Cannot go back more than 1024 blocks")]
    TooManyBlocksBack,

    #[error("Replacement transaction is underpriced")]
    ReplacementTransactionUnderpriced,

    #[error("Transaction fee below minimum")]
    FeeBelowMinimum,
}

impl StarknetApiError {
    /// Create a new unexpected error with the given reason.
    pub fn unexpected<T: ToString>(reason: T) -> Self {
        StarknetApiError::UnexpectedError(UnexpectedErrorData { reason: reason.to_string() })
    }

    /// Create a new transaction execution error with the given transaction index and reason.
    pub fn transaction_execution_error<T: ToString>(transaction_index: u64, reason: T) -> Self {
        StarknetApiError::TransactionExecutionError(TransactionExecutionErrorData {
            execution_error: reason.to_string(),
            transaction_index,
        })
    }

    /// Returns the error code.
    pub fn code(&self) -> i32 {
        match self {
            StarknetApiError::FailedToReceiveTxn => 1,
            StarknetApiError::ContractNotFound => 20,
            StarknetApiError::EntrypointNotFound => 21,
            StarknetApiError::InvalidCallData => 22,
            StarknetApiError::BlockNotFound => 24,
            StarknetApiError::InvalidTxnIndex => 27,
            StarknetApiError::ClassHashNotFound => 28,
            StarknetApiError::TxnHashNotFound => 29,
            StarknetApiError::PageSizeTooBig { .. } => 31,
            StarknetApiError::NoBlocks => 32,
            StarknetApiError::InvalidContinuationToken => 33,
            StarknetApiError::TooManyKeysInFilter => 34,
            StarknetApiError::FailedToFetchPendingTransactions => 38,
            StarknetApiError::ContractError { .. } => 40,
            StarknetApiError::TransactionExecutionError { .. } => 41,
            StarknetApiError::StorageProofNotSupported { .. } => 42,
            StarknetApiError::InvalidContractClass => 50,
            StarknetApiError::ClassAlreadyDeclared => 51,
            StarknetApiError::InvalidTransactionNonce { .. } => 52,
            StarknetApiError::InsufficientResourcesForValidate => 53,
            StarknetApiError::InsufficientAccountBalance => 54,
            StarknetApiError::ValidationFailure { .. } => 55,
            StarknetApiError::CompilationFailed { .. } => 56,
            StarknetApiError::ContractClassSizeIsTooLarge => 57,
            StarknetApiError::NonAccount => 58,
            StarknetApiError::DuplicateTransaction => 59,
            StarknetApiError::CompiledClassHashMismatch => 60,
            StarknetApiError::UnsupportedTransactionVersion => 61,
            StarknetApiError::UnsupportedContractClassVersion => 62,
            StarknetApiError::UnexpectedError { .. } => 63,
            StarknetApiError::ReplacementTransactionUnderpriced => 64,
            StarknetApiError::FeeBelowMinimum => 65,
            StarknetApiError::InvalidSubscriptionId => 66,
            StarknetApiError::TooManyAddressesInFilter => 67,
            StarknetApiError::TooManyBlocksBack => 68,
            StarknetApiError::ProofLimitExceeded { .. } => 1000,
        }
    }

    /// Returns the error message.
    pub fn message(&self) -> String {
        self.to_string()
    }

    /// Returns the error data.
    pub fn data(&self) -> Option<serde_json::Value> {
        match self {
            StarknetApiError::ContractError { .. }
            | StarknetApiError::PageSizeTooBig { .. }
            | StarknetApiError::UnexpectedError { .. }
            | StarknetApiError::CompilationFailed { .. }
            | StarknetApiError::ProofLimitExceeded { .. }
            | StarknetApiError::StorageProofNotSupported { .. }
            | StarknetApiError::TransactionExecutionError { .. } => Some(serde_json::json!(self)),

            StarknetApiError::InvalidTransactionNonce(InvalidTransactionNonceData { reason })
            | StarknetApiError::ValidationFailure(ValidationFailureData { reason }) => {
                Some(Value::String(reason.to_string()))
            }
            _ => None,
        }
    }
}

/// Data for the [`StarknetApiError::PageSizeTooBig`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PageSizeTooBigData {
    /// The user requested page size.
    pub requested: u64,
    /// The maximum allowed page size.
    pub max_allowed: u64,
}

/// Data for the [`StarknetApiError::ContractError`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContractErrorData {
    pub revert_error: String,
}

/// Data for the [`StarknetApiError::TransactionExecutionError`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransactionExecutionErrorData {
    /// The index of the first transaction failing in a sequence of given transactions.
    pub transaction_index: u64,
    /// The revert error with the execution trace up to the point of failure.
    pub execution_error: String,
}

/// Data for the [`StarknetApiError::StorageProofNotSupported`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageProofNotSupportedData {
    /// The oldest block whose storage proof can be obtained.
    pub oldest_block: BlockNumber,
    /// The block of the storage proof that is being requested.
    pub requested_block: BlockNumber,
}

/// Data for the [`StarknetApiError::InvalidTransactionNonce`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InvalidTransactionNonceData {
    pub reason: String,
}

/// Data for the [`StarknetApiError::UnexpectedError`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnexpectedErrorData {
    pub reason: String,
}

/// Data for the [`StarknetApiError::ProofLimitExceeded`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProofLimitExceededData {
    /// The limit for the total number of keys that can be specified in a single request.
    pub limit: u64,
    /// The total number of keys that is being requested.
    pub total: u64,
}

/// Data for the [`StarknetApiError::ValidationFailure`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ValidationFailureData {
    /// The reason for the  failure.
    pub reason: String,
}

/// Data for the [`StarknetApiError::CompilationFailed`] error.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompilationFailedData {
    /// The reason for the compilation failure.
    pub reason: String,
}

// Implementations from/to other error types
mod impls {

    use jsonrpsee::types::ErrorObjectOwned;
    use katana_pool_api::validation::InvalidTransactionError;
    use katana_pool_api::PoolError;
    use katana_primitives::event::ContinuationTokenError;
    use katana_provider_api::ProviderError;
    use starknet::core::types::StarknetError as StarknetRsError;
    use starknet::providers::ProviderError as StarknetRsProviderError;

    use super::{
        CompilationFailedData, ContractErrorData, InvalidTransactionNonceData, PageSizeTooBigData,
        StarknetApiError, StorageProofNotSupportedData, ValidationFailureData,
    };
    use crate::error::starknet::{
        ProofLimitExceededData, TransactionExecutionErrorData, UnexpectedErrorData,
    };

    impl From<ErrorObjectOwned> for StarknetApiError {
        fn from(err: ErrorObjectOwned) -> Self {
            match err.code() {
                1 => Self::FailedToReceiveTxn,
                20 => Self::ContractNotFound,
                21 => Self::EntrypointNotFound,
                22 => Self::InvalidCallData,
                24 => Self::BlockNotFound,
                27 => Self::InvalidTxnIndex,
                28 => Self::ClassHashNotFound,
                29 => Self::TxnHashNotFound,
                31 => {
                    if let Some(data) = err.data() {
                        if let Ok(data) = serde_json::from_str::<PageSizeTooBigData>(data.get()) {
                            return Self::PageSizeTooBig(data);
                        }
                    }

                    Self::PageSizeTooBig(Default::default())
                }
                32 => Self::NoBlocks,
                33 => Self::InvalidContinuationToken,
                34 => Self::TooManyKeysInFilter,
                38 => Self::FailedToFetchPendingTransactions,
                40 => {
                    let data = if let Some(data) = err.data() {
                        serde_json::from_str::<ContractErrorData>(data.get()).unwrap_or_default()
                    } else {
                        ContractErrorData::default()
                    };

                    Self::ContractError(data)
                }
                41 => {
                    let data = if let Some(data) = err.data() {
                        serde_json::from_str::<TransactionExecutionErrorData>(data.get())
                            .unwrap_or_default()
                    } else {
                        TransactionExecutionErrorData::default()
                    };

                    Self::TransactionExecutionError(data)
                }
                42 => {
                    if let Some(data) = err.data() {
                        if let Ok(data) =
                            serde_json::from_str::<StorageProofNotSupportedData>(data.get())
                        {
                            return Self::StorageProofNotSupported(data);
                        }
                    }

                    Self::StorageProofNotSupported(StorageProofNotSupportedData::default())
                }
                50 => Self::InvalidContractClass,
                51 => Self::ClassAlreadyDeclared,
                52 => {
                    let data = if let Some(value) = err.data() {
                        serde_json::from_str::<InvalidTransactionNonceData>(value.get())
                            .unwrap_or_default()
                    } else {
                        InvalidTransactionNonceData::default()
                    };

                    Self::InvalidTransactionNonce(data)
                }
                53 => Self::InsufficientResourcesForValidate,
                54 => Self::InsufficientAccountBalance,
                55 => {
                    let data = if let Some(data) = err.data() {
                        serde_json::from_str::<ValidationFailureData>(data.get())
                            .unwrap_or_default()
                    } else {
                        ValidationFailureData::default()
                    };

                    Self::ValidationFailure(data)
                }
                56 => {
                    let data = if let Some(data) = err.data() {
                        serde_json::from_str::<CompilationFailedData>(data.get())
                            .unwrap_or_default()
                    } else {
                        CompilationFailedData::default()
                    };

                    Self::CompilationFailed(data)
                }
                57 => Self::ContractClassSizeIsTooLarge,
                58 => Self::NonAccount,
                59 => Self::DuplicateTransaction,
                60 => Self::CompiledClassHashMismatch,
                61 => Self::UnsupportedTransactionVersion,
                62 => Self::UnsupportedContractClassVersion,
                63 => {
                    let data = if let Some(data) = err.data() {
                        serde_json::from_str::<UnexpectedErrorData>(data.get()).unwrap_or_default()
                    } else {
                        UnexpectedErrorData::default()
                    };

                    Self::UnexpectedError(data)
                }
                64 => Self::ReplacementTransactionUnderpriced,
                65 => Self::FeeBelowMinimum,
                66 => Self::InvalidSubscriptionId,
                67 => Self::TooManyAddressesInFilter,
                68 => Self::TooManyBlocksBack,
                1000 => {
                    let data = if let Some(data) = err.data() {
                        serde_json::from_str::<ProofLimitExceededData>(data.get())
                            .unwrap_or_default()
                    } else {
                        ProofLimitExceededData::default()
                    };

                    Self::ProofLimitExceeded(data)
                }

                _ => Self::unexpected(err),
            }
        }
    }

    impl From<StarknetApiError> for ErrorObjectOwned {
        fn from(err: StarknetApiError) -> Self {
            ErrorObjectOwned::owned(err.code(), err.message(), err.data())
        }
    }

    impl From<ProviderError> for StarknetApiError {
        fn from(value: ProviderError) -> Self {
            StarknetApiError::unexpected(value)
        }
    }

    impl From<ContinuationTokenError> for StarknetApiError {
        fn from(value: ContinuationTokenError) -> Self {
            match value {
                ContinuationTokenError::InvalidToken => StarknetApiError::InvalidContinuationToken,
                ContinuationTokenError::ParseFailed(e) => StarknetApiError::unexpected(e),
            }
        }
    }

    impl From<anyhow::Error> for StarknetApiError {
        fn from(value: anyhow::Error) -> Self {
            StarknetApiError::unexpected(value)
        }
    }

    impl From<PoolError> for StarknetApiError {
        fn from(error: PoolError) -> Self {
            match error {
                PoolError::InvalidTransaction(err) => err.into(),
                PoolError::Internal(err) => StarknetApiError::unexpected(err),
            }
        }
    }

    impl From<Box<InvalidTransactionError>> for StarknetApiError {
        fn from(error: Box<InvalidTransactionError>) -> Self {
            match error.as_ref() {
                InvalidTransactionError::InsufficientFunds { .. } => {
                    Self::InsufficientAccountBalance
                }
                InvalidTransactionError::ClassAlreadyDeclared { .. } => Self::ClassAlreadyDeclared,
                InvalidTransactionError::InsufficientIntrinsicFee(..) => {
                    Self::InsufficientResourcesForValidate
                }
                InvalidTransactionError::NonAccount { .. } => Self::NonAccount,
                InvalidTransactionError::InvalidNonce { .. } => {
                    Self::InvalidTransactionNonce(InvalidTransactionNonceData {
                        reason: error.to_string(),
                    })
                }
                InvalidTransactionError::ValidationFailure { error, .. } => {
                    Self::ValidationFailure(ValidationFailureData { reason: error.to_string() })
                }
            }
        }
    }

    // ---- Forking client error conversion

    impl From<StarknetRsError> for StarknetApiError {
        fn from(value: StarknetRsError) -> Self {
            match value {
                StarknetRsError::FeeBelowMinimum => Self::FeeBelowMinimum,
                StarknetRsError::ReplacementTransactionUnderpriced => {
                    Self::ReplacementTransactionUnderpriced
                }
                StarknetRsError::FailedToReceiveTransaction => Self::FailedToReceiveTxn,
                StarknetRsError::NoBlocks => Self::NoBlocks,
                StarknetRsError::NonAccount => Self::NonAccount,
                StarknetRsError::BlockNotFound => Self::BlockNotFound,
                StarknetRsError::PageSizeTooBig => {
                    Self::PageSizeTooBig(PageSizeTooBigData { requested: 0, max_allowed: 0 })
                }
                StarknetRsError::DuplicateTx => Self::DuplicateTransaction,
                StarknetRsError::ContractNotFound => Self::ContractNotFound,
                StarknetRsError::ClassHashNotFound => Self::ClassHashNotFound,
                StarknetRsError::TooManyKeysInFilter => Self::TooManyKeysInFilter,
                StarknetRsError::InvalidTransactionIndex => Self::InvalidTxnIndex,
                StarknetRsError::TransactionHashNotFound => Self::TxnHashNotFound,
                StarknetRsError::ClassAlreadyDeclared => Self::ClassAlreadyDeclared,
                StarknetRsError::UnexpectedError(reason) => Self::unexpected(reason),
                StarknetRsError::InvalidContinuationToken => Self::InvalidContinuationToken,
                StarknetRsError::UnsupportedTxVersion => Self::UnsupportedTransactionVersion,
                StarknetRsError::CompiledClassHashMismatch => Self::CompiledClassHashMismatch,
                StarknetRsError::CompilationFailed(reason) => {
                    Self::CompilationFailed(CompilationFailedData { reason })
                }
                StarknetRsError::InsufficientAccountBalance => Self::InsufficientAccountBalance,
                StarknetRsError::ValidationFailure(reason) => {
                    Self::ValidationFailure(ValidationFailureData { reason })
                }
                StarknetRsError::ContractClassSizeIsTooLarge => Self::ContractClassSizeIsTooLarge,
                StarknetRsError::EntrypointNotFound => Self::EntrypointNotFound,
                StarknetRsError::ContractError(..) => {
                    Self::ContractError(ContractErrorData { revert_error: String::new() })
                }
                StarknetRsError::TransactionExecutionError(data) => {
                    Self::transaction_execution_error(data.transaction_index, String::new())
                }
                StarknetRsError::InvalidTransactionNonce(reason) => {
                    Self::InvalidTransactionNonce(InvalidTransactionNonceData { reason })
                }
                StarknetRsError::UnsupportedContractClassVersion => {
                    Self::UnsupportedContractClassVersion
                }
                StarknetRsError::NoTraceAvailable(_) => Self::unexpected("No trace available"),
                StarknetRsError::StorageProofNotSupported => {
                    Self::StorageProofNotSupported(StorageProofNotSupportedData {
                        oldest_block: 0,
                        requested_block: 0,
                    })
                }
                StarknetRsError::InsufficientResourcesForValidate => {
                    Self::InsufficientResourcesForValidate
                }
                StarknetRsError::InvalidSubscriptionId => Self::InvalidSubscriptionId,
                StarknetRsError::TooManyAddressesInFilter => Self::TooManyAddressesInFilter,
                StarknetRsError::TooManyBlocksBack => Self::TooManyBlocksBack,
            }
        }
    }

    impl From<StarknetRsProviderError> for StarknetApiError {
        fn from(value: StarknetRsProviderError) -> Self {
            match value {
                StarknetRsProviderError::StarknetError(error) => error.into(),
                StarknetRsProviderError::Other(error) => Self::unexpected(error),
                StarknetRsProviderError::ArrayLengthMismatch => {
                    Self::unexpected("Forking client: Array length mismatch")
                }
                StarknetRsProviderError::RateLimited => {
                    Self::unexpected("Forking client: Rate limited")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee::types::ErrorObjectOwned;
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    #[rustfmt::skip]
    #[rstest]
    #[case(StarknetApiError::NoBlocks, 32, "There are no blocks")]
    #[case(StarknetApiError::BlockNotFound, 24, "Block not found")]
    #[case(StarknetApiError::InvalidCallData, 22, "Invalid call data")]
    #[case(StarknetApiError::ContractNotFound, 20, "Contract not found")]
    #[case(StarknetApiError::ClassHashNotFound, 28, "Class hash not found")]
    #[case(StarknetApiError::TxnHashNotFound, 29, "Transaction hash not found")]
    #[case(StarknetApiError::ClassAlreadyDeclared, 51, "Class already declared")]
    #[case(StarknetApiError::InvalidContractClass, 50, "Invalid contract class")]
    #[case(StarknetApiError::FailedToReceiveTxn, 1, "Failed to write transaction")]
    #[case(StarknetApiError::EntrypointNotFound, 21, "Requested entrypoint does not exist in the contract")]
    #[case(StarknetApiError::NonAccount, 58, "Sender address in not an account contract")]
    #[case(StarknetApiError::InvalidTxnIndex, 27, "Invalid transaction index in a block")]
    #[case(StarknetApiError::TooManyKeysInFilter, 34, "Too many keys provided in a filter")]
    #[case(StarknetApiError::ContractClassSizeIsTooLarge, 57, "Contract class size is too large")]
    #[case(StarknetApiError::FailedToFetchPendingTransactions, 38, "Failed to fetch pending transactions")]
    #[case(StarknetApiError::UnsupportedTransactionVersion, 61, "The transaction version is not supported")]
    #[case(StarknetApiError::UnsupportedContractClassVersion, 62, "The contract class version is not supported")]
    #[case(StarknetApiError::InvalidContinuationToken, 33, "The supplied continuation token is invalid or unknown")]
    #[case(StarknetApiError::DuplicateTransaction, 59, "A transaction with the same hash already exists in the mempool")]
    #[case(StarknetApiError::InsufficientAccountBalance, 54, "Account balance is smaller than the transaction's max_fee")]
    #[case(StarknetApiError::CompiledClassHashMismatch, 60, "The compiled class hash did not match the one supplied in the transaction")]
    #[case(StarknetApiError::InsufficientResourcesForValidate, 53, "The transaction's resources don't cover validation or the minimal transaction fee")]
    #[case(StarknetApiError::InvalidSubscriptionId, 66, "Invalid subscription id")]
    #[case(StarknetApiError::TooManyAddressesInFilter, 67, "Too many addresses in filter sender_address filter")]
    #[case(StarknetApiError::TooManyBlocksBack, 68, "Cannot go back more than 1024 blocks")]
    fn test_starknet_api_error_to_error_conversion_data_none(
        #[case] starknet_error: StarknetApiError,
        #[case] expected_code: i32,
        #[case] expected_message: &str,
    ) {
        let err: ErrorObjectOwned = starknet_error.into();
        assert_eq!(err.code(), expected_code);
        assert_eq!(err.message(), expected_message);
	    assert!(err.data().is_none(), "data should be None");
    }

    #[rstest]
    #[case(
        StarknetApiError::ContractError(ContractErrorData {
            revert_error: "Contract error message".to_string(),
        }),
        40,
        "Contract error",
        json!({
            "revert_error": "Contract error message".to_string()
        }),
    )]
    #[case(
        StarknetApiError::TransactionExecutionError(TransactionExecutionErrorData {
            transaction_index: 1,
            execution_error: "Transaction execution error message".to_string(),
        }),
        41,
        "Transaction execution error",
        json!({
            "transaction_index": 1,
            "execution_error": "Transaction execution error message".to_string()
        }),
    )]
    #[case(
        StarknetApiError::UnexpectedError(UnexpectedErrorData {
            reason: "Unexpected error reason".to_string(),
        }),
        63,
        "An unexpected error occurred",
        json!({
            "reason": "Unexpected error reason".to_string()
        }),
    )]
    #[case(
    	StarknetApiError::InvalidTransactionNonce(InvalidTransactionNonceData {
     		reason: "Wrong nonce".to_string()
    	}),
     	52,
      	"Invalid transaction nonce",
       	Value::String("Wrong nonce".to_string())
    )]
    #[case(
    	StarknetApiError::CompilationFailed(CompilationFailedData {
     		reason: "Failed to compile".to_string()
    	}),
     	56,
      	"Compilation failed",
       json!({
           "reason": "Failed to compile".to_string()
       }),
    )]
    #[case(
    	StarknetApiError::ValidationFailure(ValidationFailureData {
     		reason: "Invalid signature".to_string()
    	}),
     	55,
      	"Account validation failed",
       	Value::String("Invalid signature".to_string())
    )]
    #[case(
    	StarknetApiError::PageSizeTooBig(PageSizeTooBigData {
     		requested: 1000,
       		max_allowed: 500
    	}),
      	31,
       	"Requested page size is too big",
        json!({
        	"requested": 1000,
         	"max_allowed": 500
        }),
    )]
    #[case(
    	StarknetApiError::StorageProofNotSupported(StorageProofNotSupportedData {
     		oldest_block: 10,
       		requested_block: 9
    	}),
      	42,
       	"The node doesn't support storage proofs for blocks that are too far in the past",
        json!({
        	"oldest_block": 10,
         	"requested_block": 9
        }),
    )]
    #[case(
    	StarknetApiError::ProofLimitExceeded(ProofLimitExceededData {
     		limit: 5,
       		total: 10
    	}),
      	1000,
       	"Proof limit exceeded",
        json!({
        	"limit": 5,
         	"total": 10
        }),
    )]
    fn test_starknet_api_error_to_error_conversion_data_some(
        #[case] starknet_error: StarknetApiError,
        #[case] expected_code: i32,
        #[case] expected_message: &str,
        #[case] expected_data: serde_json::Value,
    ) {
        let err: ErrorObjectOwned = starknet_error.into();
        assert_eq!(err.code(), expected_code);
        assert_eq!(err.message(), expected_message);
        assert_eq!(err.data().unwrap().to_string(), expected_data.to_string(), "data should exist");
    }
}
