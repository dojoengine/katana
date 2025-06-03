use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use katana_executor::implementation::blockifier::blockifier::blockifier::stateful_validator::{
    StatefulValidator, StatefulValidatorError,
};
use katana_executor::implementation::blockifier::blockifier::blockifier::transaction_executor::TransactionExecutorError;
use katana_executor::implementation::blockifier::blockifier::state::cached_state::CachedState;
use katana_executor::implementation::blockifier::blockifier::transaction::errors::{
    TransactionExecutionError, TransactionFeeError, TransactionPreValidationError,
};
use katana_executor::implementation::blockifier::blockifier::transaction::transaction_execution::Transaction;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::state::StateProviderDb;
use katana_executor::implementation::blockifier::utils::{
    block_context_from_envs, to_address, to_executor_tx,
};
use katana_executor::ExecutionFlags;
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::env::{BlockEnv, CfgEnv};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash};
use katana_primitives::Felt;
use katana_provider::error::ProviderError;
use katana_provider::traits::state::StateProvider;
use parking_lot::Mutex;

use super::{Error, InvalidTransactionError, ValidationOutcome, ValidationResult, Validator};
use crate::tx::PoolTransaction;
use crate::validation::error::{InsufficientFundsError, IntrinsicFeeTooLowError};

#[derive(Debug, Clone)]
pub struct TxValidator {
    inner: Arc<Mutex<Inner>>,
    permit: Arc<Mutex<()>>,
}

struct Inner {
    // execution context
    cfg_env: CfgEnv,
    block_env: BlockEnv,
    execution_flags: ExecutionFlags,
    state: Arc<Box<dyn StateProvider>>,
    pool_nonces: HashMap<ContractAddress, Nonce>,
}

impl TxValidator {
    pub fn new(
        state: Box<dyn StateProvider>,
        execution_flags: ExecutionFlags,
        cfg_env: CfgEnv,
        block_env: BlockEnv,
        permit: Arc<Mutex<()>>,
    ) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            cfg_env,
            block_env,
            execution_flags,
            state: Arc::new(state),
            pool_nonces: HashMap::new(),
        }));
        Self { permit, inner }
    }

    /// Reset the state of the validator with the given params. This method is used to update the
    /// validator's state with a new state and block env after a block is mined.
    pub fn update(&self, new_state: Box<dyn StateProvider>, block_env: BlockEnv) {
        let mut this = self.inner.lock();
        this.block_env = block_env;
        this.state = Arc::new(new_state);
    }

    // NOTE:
    // If you check the get_nonce method of StatefulValidator in blockifier, under the hood it
    // unwraps the Option to get the state of the TransactionExecutor struct. StatefulValidator
    // guaranteees that the state will always be present so it is safe to uwnrap. However, this
    // safety is not guaranteed by TransactionExecutor itself.
    pub fn pool_nonce(&self, address: ContractAddress) -> Result<Option<Nonce>, ProviderError> {
        let this = self.inner.lock();
        match this.pool_nonces.get(&address) {
            Some(nonce) => Ok(Some(*nonce)),
            None => Ok(this.state.nonce(address)?),
        }
    }
}

impl Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("cfg_env", &self.cfg_env)
            .field("block_env", &self.block_env)
            .field("execution_flags", &self.execution_flags)
            .field("state", &"..")
            .field("pool_nonces", &self.pool_nonces)
            .finish()
    }
}

impl Inner {
    // Prepare the stateful validator with the current state and block env to be used
    // for transaction validation.
    fn prepare(&self) -> StatefulValidator<StateProviderDb<'static>> {
        let state = Box::new(self.state.clone());
        let class_cache = ClassCache::global().clone();
        let state_provider = StateProviderDb::new_with_class_cache(state, class_cache);

        let cached_state = CachedState::new(state_provider);
        let context = block_context_from_envs(&self.block_env, &self.cfg_env);

        StatefulValidator::create(cached_state, context)
    }
}

impl Validator for TxValidator {
    type Transaction = ExecutableTxWithHash;

    fn validate(&self, tx: Self::Transaction) -> ValidationResult<Self::Transaction> {
        let _permit = self.permit.lock();
        let mut this = self.inner.lock();

        let tx_nonce = tx.nonce();
        let address = tx.sender();

        // For declare transactions, perform a static check if there's already an existing class
        // with the same hash.
        if let ExecutableTx::Declare(ref declare_tx) = tx.transaction {
            let class_hash = declare_tx.class_hash();
            let class = this.state.class(class_hash).map_err(|e| Error::new(tx.hash, e.into()))?;

            // Return an error if the class already exists.
            if class.is_some() {
                let error = InvalidTransactionError::ClassAlreadyDeclared { class_hash };
                return Ok(ValidationOutcome::Invalid { tx, error });
            }
        }

        // Get the current nonce of the account from the pool or the state
        let current_nonce = if let Some(nonce) = this.pool_nonces.get(&address) {
            *nonce
        } else {
            this.state.nonce(address).unwrap().unwrap_or_default()
        };

        // Check if the transaction nonce is higher than the current account nonce,
        // if yes, dont't run its validation logic and tag it as a dependent tx.
        if tx_nonce > current_nonce {
            return Ok(ValidationOutcome::Dependent { current_nonce, tx_nonce, tx });
        }

        // Check if validation of an invoke transaction should be skipped due to deploy_account not
        // being proccessed yet. This feature is used to improve UX for users sending
        // deploy_account + invoke at once.
        let skip_validate = match tx.transaction {
            // we skip validation for invoke tx with nonce 1 and nonce 0 in the state, this
            ExecutableTx::DeployAccount(_) | ExecutableTx::Declare(_) => false,
            // we skip validation for invoke tx with nonce 1 and nonce 0 in the state, this
            _ => tx.nonce() == Nonce::ONE && current_nonce == Nonce::ZERO,
        };

        // prepare a stateful validator and run the account validation logic (ie __validate__
        // entrypoint)
        let result = validate(
            this.prepare(),
            tx,
            !this.execution_flags.account_validation() || skip_validate,
            !this.execution_flags.fee(),
        );

        match result {
            res @ Ok(ValidationOutcome::Valid { .. }) => {
                // update the nonce of the account in the pool only for valid tx
                let updated_nonce = current_nonce + Felt::ONE;
                this.pool_nonces.insert(address, updated_nonce);
                res
            }
            _ => result,
        }
    }
}

// perform validation on the pool transaction using the provided stateful validator
fn validate(
    mut validator: StatefulValidator<StateProviderDb<'static>>,
    pool_tx: ExecutableTxWithHash,
    skip_validate: bool,
    skip_fee_check: bool,
) -> ValidationResult<ExecutableTxWithHash> {
    let flags = ExecutionFlags::new()
        .with_account_validation(!skip_validate)
        .with_fee(!skip_fee_check)
        .with_nonce_check(false);

    match to_executor_tx(pool_tx.clone(), flags) {
        Transaction::Account(tx) => match validator.perform_validations(tx) {
            Ok(()) => Ok(ValidationOutcome::Valid(pool_tx)),
            Err(e) => match map_invalid_tx_err(e) {
                Ok(error) => Ok(ValidationOutcome::Invalid { tx: pool_tx, error }),
                Err(error) => Err(Error { hash: pool_tx.hash, error }),
            },
        },

        // we skip validation for L1HandlerTransaction
        Transaction::L1Handler(_) => Ok(ValidationOutcome::Valid(pool_tx)),
    }
}

fn map_invalid_tx_err(
    err: StatefulValidatorError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error>> {
    match err {
        StatefulValidatorError::TransactionExecutorError(err) => map_executor_err(err),
        StatefulValidatorError::TransactionExecutionError(err) => map_execution_err(err),
        StatefulValidatorError::TransactionPreValidationError(err) => map_pre_validation_err(err),
        _ => Err(Box::new(err)),
    }
}

fn map_fee_err(
    err: TransactionFeeError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error>> {
    match err {
        TransactionFeeError::GasBoundsExceedBalance {
            resource,
            max_amount,
            max_price,
            balance,
        } => {
            let max_amount = max_amount.0;
            let max_price = max_price.0;
            let balance: Felt = balance.into();

            let error = InsufficientFundsError::L1GasBoundsExceedFunds {
                balance,
                resource,
                max_price,
                max_amount,
            };

            Ok(InvalidTransactionError::InsufficientFunds(error))
        }

        TransactionFeeError::ResourcesBoundsExceedBalance { .. } => {
            let error =
                InsufficientFundsError::ResourceBoundsExceedFunds { error: err.to_string() };
            Ok(InvalidTransactionError::InsufficientFunds(error))
        }

        TransactionFeeError::MaxFeeExceedsBalance { max_fee, balance } => {
            let max_fee = max_fee.0;
            let balance = balance.into();

            let error = InsufficientFundsError::MaxFeeExceedsFunds { max_fee, balance };
            Ok(InvalidTransactionError::InsufficientFunds(error))
        }

        TransactionFeeError::MaxFeeTooLow { min_fee, max_fee } => {
            let max_fee = max_fee.0;
            let min_fee = min_fee.0;
            Ok(InvalidTransactionError::IntrinsicFeeTooLow(IntrinsicFeeTooLowError::MaxFee {
                max_fee,
                min: min_fee,
            }))
        }

        TransactionFeeError::InsufficientResourceBounds { errors } => {
            let error = errors.iter().map(|e| format!("{}", e)).collect::<Vec<_>>().join("\n");
            Ok(InvalidTransactionError::IntrinsicFeeTooLow(
                IntrinsicFeeTooLowError::ResourceBounds { error },
            ))
        }

        _ => Err(Box::new(err)),
    }
}

fn map_executor_err(
    err: TransactionExecutorError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error>> {
    match err {
        TransactionExecutorError::TransactionExecutionError(e) => match e {
            TransactionExecutionError::TransactionPreValidationError(e) => {
                map_pre_validation_err(e)
            }

            _ => Err(Box::new(e)),
        },

        _ => Err(Box::new(err)),
    }
}

fn map_execution_err(
    err: TransactionExecutionError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error>> {
    match err {
        e @ TransactionExecutionError::ValidateTransactionError {
            storage_address,
            class_hash,
            ..
        } => {
            let address = to_address(storage_address);
            let class_hash = class_hash.0;
            let error = e.to_string();
            Ok(InvalidTransactionError::ValidationFailure { address, class_hash, error })
        }

        TransactionExecutionError::PanicInValidate { panic_reason } => {
            // TODO: maybe can remove the address and class hash?
            Ok(InvalidTransactionError::ValidationFailure {
                address: Default::default(),
                class_hash: Default::default(),
                error: panic_reason.to_string(),
            })
        }
        _ => Err(Box::new(err)),
    }
}

fn map_pre_validation_err(
    err: TransactionPreValidationError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error>> {
    match err {
        TransactionPreValidationError::InvalidNonce {
            address,
            account_nonce,
            incoming_tx_nonce,
        } => {
            let address = to_address(address);
            let current_nonce = account_nonce.0;
            let tx_nonce = incoming_tx_nonce.0;
            Ok(InvalidTransactionError::InvalidNonce { address, current_nonce, tx_nonce })
        }

        TransactionPreValidationError::TransactionFeeError(err) => map_fee_err(err),

        _ => Err(Box::new(err)),
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::validation::error::{IntrinsicFeeTooLowError, InvalidTransactionError};

//     #[test]
//     fn test_intrinsic_fee_too_low_legacy_formatting() {
//         // Test Legacy variant
//         let legacy_error = InvalidTransactionError::IntrinsicFeeTooLow {
//             kind: IntrinsicFeeTooLowError::MaxFee { min: 2000, max_fee: 1500 },
//         };
//         assert_eq!(legacy_error.to_string(), "Max fee (1500) is too low. Minimum fee: 2000.");
//     }

//     #[test]
//     fn test_intrinsic_fee_too_low_resource_bounds_formatting() {
//         // Test ResourceBounds variant
//         let resource_error = InvalidTransactionError::IntrinsicFeeTooLow {
//             kind: IntrinsicFeeTooLowError::ResourceBounds {
//                 error: "L1 gas bounds exceed balance".to_string(),
//             },
//         };
//         assert_eq!(
//             resource_error.to_string(),
//             "Resource bounds were not satisfied: L1 gas bounds exceed balance"
//         );
//     }

//     #[test]
//     fn test_intrinsic_fee_too_low_resource_bounds_multiple_errors() {
//         // Test ResourceBounds variant with multiple errors joined
//         let errors = vec!["L1 gas bounds exceed balance", "L2 gas bounds exceed balance"];
//         let error_string = errors.iter().map(|e| format!("{}",
// e)).collect::<Vec<_>>().join("\n");

//         let resource_error = InvalidTransactionError::IntrinsicFeeTooLow {
//             kind: IntrinsicFeeTooLowError::ResourceBounds { error: error_string },
//         };

//         let expected = "Resource bounds were not satisfied: L1 gas bounds exceed balance\nL2 gas
// bounds exceed balance";         assert_eq!(resource_error.to_string(), expected);
//     }
// }
