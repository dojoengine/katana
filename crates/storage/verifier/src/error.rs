//! Error types for the verification process.

use thiserror::Error;

/// Errors that can occur during database verification.
#[derive(Error, Debug)]
pub enum VerificationError {
    #[error("Block count mismatch: expected {expected}, found {actual}")]
    BlockCountMismatch { expected: u64, actual: u64 },

    #[error("Transaction count mismatch at block {block_number}: expected {expected}, found {actual}")]
    TransactionCountMismatch { block_number: u64, expected: usize, actual: usize },

    #[error("Block hash mismatch at block {block_number}: computed {computed}, stored {stored}")]
    BlockHashMismatch { block_number: u64, computed: String, stored: String },

    #[error("State root mismatch at block {block_number}: header {header}, computed {computed}")]
    StateRootMismatch { block_number: u64, header: String, computed: String },

    #[error("Transaction commitment mismatch at block {block_number}: header {header}, computed {computed}")]
    TransactionCommitmentMismatch { block_number: u64, header: String, computed: String },

    #[error("Receipt commitment mismatch at block {block_number}: header {header}, computed {computed}")]
    ReceiptCommitmentMismatch { block_number: u64, header: String, computed: String },

    #[error("Events commitment mismatch at block {block_number}: header {header}, computed {computed}")]
    EventsCommitmentMismatch { block_number: u64, header: String, computed: String },

    #[error("Parent hash mismatch at block {block_number}: expected {expected}, found {actual}")]
    ParentHashMismatch { block_number: u64, expected: String, actual: String },

    #[error("Block sequence broken: gap between block {previous} and {current}")]
    BlockSequenceGap { previous: u64, current: u64 },

    #[error("Invalid block range: start={start}, end={end}, latest={latest}")]
    InvalidBlockRange { start: u64, end: u64, latest: u64 },

    #[error("Missing block data at block {block_number}: {data_type}")]
    MissingBlockData { block_number: u64, data_type: String },

    #[error("Contract storage inconsistency at block {block_number}, contract {contract}, key {key}")]
    StorageInconsistency { block_number: u64, contract: String, key: String },

    #[error("Class hash mismatch for contract {contract} at block {block_number}: expected {expected}, found {actual}")]
    ClassHashMismatch { block_number: u64, contract: String, expected: String, actual: String },

    #[error("Execution result mismatch at block {block_number}, transaction {tx_hash}: expected {expected}, got {actual}")]
    ExecutionResultMismatch { block_number: u64, tx_hash: String, expected: String, actual: String },

    #[error("Database provider error: {0}")]
    DatabaseProvider(#[from] anyhow::Error),
}

/// Result type for verification operations.
pub type VerificationResult<T> = std::result::Result<T, VerificationError>;
