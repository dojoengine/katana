//! Error handling for gRPC services.
//!
//! This module provides conversion from Starknet API errors to gRPC status codes.

use katana_rpc_api::error::starknet::StarknetApiError;
use tonic::{Code, Status};

/// Converts a [`StarknetApiError`] to a [`tonic::Status`].
///
/// This mapping follows gRPC best practices for error codes:
/// - NOT_FOUND: Resource doesn't exist (block, transaction, contract, class)
/// - INVALID_ARGUMENT: Invalid input parameters
/// - FAILED_PRECONDITION: Operation cannot be performed in current state
/// - RESOURCE_EXHAUSTED: Limits exceeded
/// - INTERNAL: Unexpected internal errors
/// - UNIMPLEMENTED: Unsupported features
pub fn to_status(err: StarknetApiError) -> Status {
    match err {
        // Not found errors -> NOT_FOUND
        StarknetApiError::BlockNotFound => Status::new(Code::NotFound, "Block not found"),
        StarknetApiError::TxnHashNotFound => Status::new(Code::NotFound, "Transaction not found"),
        StarknetApiError::ContractNotFound => Status::new(Code::NotFound, "Contract not found"),
        StarknetApiError::ClassHashNotFound => Status::new(Code::NotFound, "Class hash not found"),
        StarknetApiError::NoBlocks => Status::new(Code::NotFound, "No blocks"),
        StarknetApiError::EntrypointNotFound => Status::new(Code::NotFound, "Entrypoint not found"),

        // Invalid argument errors -> INVALID_ARGUMENT
        StarknetApiError::InvalidTxnIndex => {
            Status::new(Code::InvalidArgument, "Invalid transaction index")
        }
        StarknetApiError::InvalidCallData => Status::new(Code::InvalidArgument, "Invalid calldata"),
        StarknetApiError::InvalidContractClass => {
            Status::new(Code::InvalidArgument, "Invalid contract class")
        }
        StarknetApiError::InvalidContinuationToken => {
            Status::new(Code::InvalidArgument, "Invalid continuation token")
        }
        StarknetApiError::TooManyKeysInFilter => {
            Status::new(Code::InvalidArgument, "Too many keys in filter")
        }
        StarknetApiError::TooManyAddressesInFilter => {
            Status::new(Code::InvalidArgument, "Too many addresses in filter")
        }
        StarknetApiError::TooManyBlocksBack => {
            Status::new(Code::InvalidArgument, "Too many blocks back")
        }
        StarknetApiError::InvalidSubscriptionId => {
            Status::new(Code::InvalidArgument, "Invalid subscription id")
        }

        // Resource exhausted errors -> RESOURCE_EXHAUSTED
        StarknetApiError::PageSizeTooBig(data) => Status::new(
            Code::ResourceExhausted,
            format!("Page size too big: requested {}, max {}", data.requested, data.max_allowed),
        ),
        StarknetApiError::ProofLimitExceeded(data) => Status::new(
            Code::ResourceExhausted,
            format!("Proof limit exceeded: {} keys requested, limit is {}", data.total, data.limit),
        ),
        StarknetApiError::ContractClassSizeIsTooLarge => {
            Status::new(Code::ResourceExhausted, "Contract class size is too large")
        }

        // Transaction validation errors -> FAILED_PRECONDITION
        StarknetApiError::InsufficientAccountBalance => {
            Status::new(Code::FailedPrecondition, "Insufficient account balance")
        }
        StarknetApiError::InsufficientResourcesForValidate => {
            Status::new(Code::FailedPrecondition, "Insufficient resources for validation")
        }
        StarknetApiError::InvalidTransactionNonce(data) => Status::new(
            Code::FailedPrecondition,
            format!("Invalid transaction nonce: {}", data.reason),
        ),
        StarknetApiError::ValidationFailure(data) => {
            Status::new(Code::FailedPrecondition, format!("Validation failure: {}", data.reason))
        }
        StarknetApiError::NonAccount => {
            Status::new(Code::FailedPrecondition, "Sender address is not an account contract")
        }
        StarknetApiError::DuplicateTransaction => {
            Status::new(Code::FailedPrecondition, "Transaction already exists in pool")
        }
        StarknetApiError::CompiledClassHashMismatch => {
            Status::new(Code::FailedPrecondition, "Compiled class hash mismatch")
        }
        StarknetApiError::FailedToReceiveTxn => {
            Status::new(Code::FailedPrecondition, "Failed to receive transaction")
        }
        StarknetApiError::FailedToFetchPendingTransactions => {
            Status::new(Code::FailedPrecondition, "Failed to fetch pending transactions")
        }
        StarknetApiError::ReplacementTransactionUnderpriced => {
            Status::new(Code::FailedPrecondition, "Replacement transaction underpriced")
        }
        StarknetApiError::FeeBelowMinimum => {
            Status::new(Code::FailedPrecondition, "Fee below minimum")
        }

        // Unsupported errors -> UNIMPLEMENTED
        StarknetApiError::UnsupportedContractClassVersion => {
            Status::new(Code::Unimplemented, "Unsupported contract class version")
        }
        StarknetApiError::UnsupportedTransactionVersion => {
            Status::new(Code::Unimplemented, "Unsupported transaction version")
        }
        StarknetApiError::StorageProofNotSupported(data) => Status::new(
            Code::Unimplemented,
            format!(
                "Storage proof not supported: oldest block {}, requested {}",
                data.oldest_block, data.requested_block
            ),
        ),

        // Execution errors -> FAILED_PRECONDITION with details
        StarknetApiError::ContractError(data) => {
            Status::new(Code::FailedPrecondition, format!("Contract error: {}", data.revert_error))
        }
        StarknetApiError::TransactionExecutionError(data) => Status::new(
            Code::FailedPrecondition,
            format!(
                "Transaction execution error at index {}: {}",
                data.transaction_index, data.execution_error
            ),
        ),
        StarknetApiError::CompilationError(data) => Status::new(
            Code::FailedPrecondition,
            format!("Compilation failed: {}", data.compilation_error),
        ),

        // Class already declared -> ALREADY_EXISTS
        StarknetApiError::ClassAlreadyDeclared => {
            Status::new(Code::AlreadyExists, "Class already declared")
        }

        // Unexpected errors -> INTERNAL
        StarknetApiError::UnexpectedError(data) => {
            Status::new(Code::Internal, format!("Unexpected error: {}", data.reason))
        }
    }
}

/// Extension trait to easily convert Results with StarknetApiError to gRPC Results.
pub trait IntoGrpcResult<T> {
    /// Converts the result to a gRPC result.
    fn into_grpc_result(self) -> Result<T, Status>;
}

impl<T> IntoGrpcResult<T> for Result<T, StarknetApiError> {
    fn into_grpc_result(self) -> Result<T, Status> {
        self.map_err(to_status)
    }
}
