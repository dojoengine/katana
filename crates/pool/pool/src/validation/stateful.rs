use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use katana_chain_spec::ChainSpec;
use katana_executor::blockifier::blockifier::blockifier::stateful_validator::{
    StatefulValidator, StatefulValidatorError,
};
use katana_executor::blockifier::blockifier::blockifier::transaction_executor::TransactionExecutorError;
use katana_executor::blockifier::blockifier::state::cached_state::CachedState;
use katana_executor::blockifier::blockifier::transaction::errors::{
    TransactionExecutionError, TransactionFeeError, TransactionPreValidationError,
};
use katana_executor::blockifier::blockifier::transaction::transaction_execution::Transaction;
use katana_executor::blockifier::cache::ClassCache;
use katana_executor::blockifier::state::StateProviderDb;
use katana_executor::blockifier::utils::{block_context_from_envs, to_address, to_executor_tx};
use katana_executor::ExecutionFlags;
use katana_pool_api::validation::{
    Error, InsufficientFundsError, InsufficientIntrinsicFeeError, InvalidTransactionError,
    ValidationOutcome, Validator,
};
use katana_pool_api::PoolTransaction;
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::env::{BlockEnv, VersionedConstantsOverrides};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash};
use katana_primitives::Felt;
use katana_provider::api::state::StateProvider;
use parking_lot::Mutex;

use super::ValidationResult;

#[derive(Debug, Clone)]
pub struct TxValidator {
    inner: Arc<Mutex<Inner>>,
    permit: Arc<Mutex<()>>,
}

struct Inner {
    // execution context
    cfg_env: Option<VersionedConstantsOverrides>,
    block_env: BlockEnv,
    execution_flags: ExecutionFlags,
    state: Arc<Box<dyn StateProvider>>,
    pool_nonces: HashMap<ContractAddress, Nonce>,
    chain_spec: Arc<ChainSpec>,
}

impl TxValidator {
    pub fn new(
        state: Box<dyn StateProvider>,
        execution_flags: ExecutionFlags,
        cfg_env: Option<VersionedConstantsOverrides>,
        block_env: BlockEnv,
        permit: Arc<Mutex<()>>,
        chain_spec: Arc<ChainSpec>,
    ) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            cfg_env,
            block_env,
            chain_spec,
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
    fn prepare(&self) -> StatefulValidator<StateProviderDb> {
        let state = Box::new(self.state.clone());
        let class_cache = ClassCache::global().clone();
        let state_provider = StateProviderDb::new_with_class_cache(state, class_cache);

        let cached_state = CachedState::new(state_provider);
        let context =
            block_context_from_envs(&self.chain_spec, &self.block_env, self.cfg_env.as_ref());

        StatefulValidator::create(cached_state, context)
    }
}

impl Validator for TxValidator {
    type Transaction = ExecutableTxWithHash;

    fn validate(
        &self,
        tx: Self::Transaction,
    ) -> impl std::future::Future<Output = ValidationResult<Self::Transaction>> + Send {
        use tracing::Instrument;

        let inner = self.inner.clone();
        let permit = self.permit.clone();
        let tx_hash = tx.hash();

        async move {
            let _permit = permit.lock();
            let mut this = inner.lock();

            let tx_nonce = tx.nonce();
            let address = tx.sender();

            // For declare transactions, perform a static check if there's already an existing class
            // with the same hash.
            if let ExecutableTx::Declare(ref declare_tx) = tx.transaction {
                let class_hash = declare_tx.class_hash();
                let class =
                    this.state.class(class_hash).map_err(|e| Error::new(tx.hash, Box::new(e)))?;

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

            // Check if validation of an invoke transaction should be skipped due to deploy_account
            // not being proccessed yet. This feature is used to improve UX for users
            // sending deploy_account + invoke at once.
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
        .instrument(
            tracing::trace_span!(target: "pool", "pool_validate", tx_hash = format!("{:#x}", tx_hash))
        )
    }
}

// perform validation on the pool transaction using the provided stateful validator
fn validate(
    mut validator: StatefulValidator<StateProviderDb>,
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
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send + Sync + 'static>> {
    match err {
        StatefulValidatorError::StateError(err) => Err(Box::new(err)),
        StatefulValidatorError::TransactionExecutorError(err) => map_executor_err(err),
        StatefulValidatorError::TransactionExecutionError(err) => map_execution_err(err),
        StatefulValidatorError::TransactionPreValidationError(err) => map_pre_validation_err(err),
    }
}

fn map_fee_err(
    err: TransactionFeeError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send + Sync + 'static>> {
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
            Ok(InvalidTransactionError::InsufficientIntrinsicFee(
                InsufficientIntrinsicFeeError::InsufficientMaxFee { max_fee, min: min_fee },
            ))
        }

        TransactionFeeError::InsufficientResourceBounds { errors } => {
            let error = errors.iter().map(|e| format!("{e}")).collect::<Vec<_>>().join("\n");
            Ok(InvalidTransactionError::InsufficientIntrinsicFee(
                InsufficientIntrinsicFeeError::InsufficientResourceBounds { error },
            ))
        }

        _ => Err(Box::new(err)),
    }
}

fn map_executor_err(
    err: TransactionExecutorError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send + Sync + 'static>> {
    match err {
        TransactionExecutorError::TransactionExecutionError(e) => match e {
            TransactionExecutionError::TransactionFeeError(e) => map_fee_err(*e),
            TransactionExecutionError::TransactionPreValidationError(e) => {
                map_pre_validation_err(*e)
            }

            _ => Err(Box::new(e)),
        },

        _ => Err(Box::new(err)),
    }
}

fn map_execution_err(
    err: TransactionExecutionError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send + Sync + 'static>> {
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
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send + Sync + 'static>> {
    match err {
        TransactionPreValidationError::TransactionFeeError(err) => map_fee_err(*err),
        TransactionPreValidationError::StateError(err) => Err(Box::new(err)),
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
    }
}
