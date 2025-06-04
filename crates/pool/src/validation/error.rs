use katana_primitives::class::ClassHash;
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::execution::Resource;
use katana_primitives::Felt;

// TODO: figure out how to combine this with ExecutionError
#[derive(Debug, thiserror::Error)]
pub enum InvalidTransactionError {
    /// Error when the account's balance is insufficient to cover the specified transaction fee.
    #[error(transparent)]
    InsufficientFunds(#[from] InsufficientFundsError),

    /// Error when the specified transaction fee is insufficient to cover the minimum fee required
    /// to start the invocation (including the account's validation logic).
    ///
    /// It is a static check that is performed before the transaction is invoked to ensure the
    /// transaction can cover the intrinsics cost ie data availability, etc.
    ///
    /// This is different from an error due to transaction runs out of gas during execution ie.
    /// the specified max fee/resource bounds is lower than the amount needed to finish the
    /// transaction execution (either validation or execution).
    #[error(transparent)]
    InsufficientIntrinsicFee(#[from] InsufficientIntrinsicFeeError),

    /// Error when the account's validation logic fails (ie __validate__ function).
    #[error("{error}")]
    ValidationFailure {
        /// The address of the contract that failed validation.
        address: ContractAddress,
        /// The class hash of the account contract.
        class_hash: ClassHash,
        /// The error message returned by Blockifier.
        // TODO: this should be a more specific error type.
        error: String,
    },

    /// Error when the transaction's sender is not an account contract.
    #[error("Sender is not an account")]
    NonAccount {
        /// The address of the contract that is not an account.
        address: ContractAddress,
    },

    /// Error when the transaction is using a nonexpected nonce.
    #[error(
        "Invalid transaction nonce of contract at address {address}. Account nonce: \
         {current_nonce:#x}; got: {tx_nonce:#x}."
    )]
    InvalidNonce {
        /// The address of the contract that has the invalid nonce.
        address: ContractAddress,
        /// The current nonce of the sender's account.
        current_nonce: Nonce,
        /// The nonce that the tx is using.
        tx_nonce: Nonce,
    },

    /// Error when a Declare transaction is trying to declare a class that has already been
    /// declared.
    #[error("Class with hash {class_hash:#x} has already been declared.")]
    ClassAlreadyDeclared { class_hash: ClassHash },
}

/// Error related to the transaction intrinsic fee.
#[derive(Debug, thiserror::Error)]
pub enum InsufficientIntrinsicFeeError {
    /// Legacy fee validation error (for <V3 transaction).
    #[error("Max fee ({max_fee}) is too low. Minimum fee: {min}.")]
    InsufficientMaxFee {
        /// The minimum required for the transaction to be executed.
        min: u128,
        /// The specified transaction fee.
        max_fee: u128,
    },

    /// Resource bounds validation error (for V3 transaction).
    #[error("Resource bounds were not satisfied: {error}")]
    InsufficientResourceBounds {
        /// The resource bounds error details.
        error: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum InsufficientFundsError {
    /// Error when the account's balance is insufficient to cover the specified transaction fee.
    #[error("Max fee ({max_fee}) exceeds balance ({balance}).")]
    MaxFeeExceedsFunds {
        /// The specified transaction fee.
        max_fee: u128,
        /// The account's balance of the fee token.
        balance: Felt,
    },

    /// Error when the L1 gas bounds specified in the transaction exceeds the sender's balance.
    #[error(
        "Resource {resource} bounds (max amount: {max_amount}, max price): {max_price}) exceed \
         balance ({balance})."
    )]
    L1GasBoundsExceedFunds {
        /// The resource that exceeds the account's balance.
        resource: Resource,
        /// The specified amount of resource.
        max_amount: u64,
        /// The specified maximum price per unit of resource.
        max_price: u128,
        /// The account's balance.
        ///
        /// Because resource bounds are only for V3 transactions, this is the STRK fee token
        /// balance.
        balance: Felt,
    },

    // TODO: dont generalize to string
    /// Error when the resource bounds specified in the transaction exceeds the sender's balance.
    ///
    /// This is applicable only to V3 transactions that set all the gas resource bounds. Prior to
    /// 0.14.0, it is permissble to only specify L1 gas bounds, or specifies zero L2 gas and no
    /// data gas bound. But on 0.14.0, it is required to set all bounds.
    #[error("{error}")]
    ResourceBoundsExceedFunds { error: String },
}
