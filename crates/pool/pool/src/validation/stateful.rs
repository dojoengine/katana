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
        this.pool_nonces.clear();
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

        let span = tracing::trace_span!(target: "pool", "pool_validate", tx_hash = format!("{:#x}", tx_hash));
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
            // this nonce validation is also handled in this function:
            // blockifier::transaction::account_transaction::AccountTransaction::perform_pre_validation_stage
            //   |
            //   -- blockifier::transaction::account_transaction::AccountTransaction::handle_nonce
            //
            // but we're handle this here to fail early
            if tx_nonce < current_nonce {
                return Ok(ValidationOutcome::Invalid {
                    tx,
                    error: InvalidTransactionError::InvalidNonce {
                        address,
                        current_nonce,
                        tx_nonce,
                    },
                });
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
        .instrument(span)
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
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send>> {
    match err {
        StatefulValidatorError::StateError(err) => Err(Box::new(err)),
        StatefulValidatorError::TransactionExecutorError(err) => map_executor_err(err),
        StatefulValidatorError::TransactionExecutionError(err) => map_execution_err(err),
        StatefulValidatorError::TransactionPreValidationError(err) => map_pre_validation_err(err),
    }
}

fn map_fee_err(
    err: TransactionFeeError,
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send>> {
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
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send>> {
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
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send>> {
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
) -> Result<InvalidTransactionError, Box<dyn std::error::Error + Send>> {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use katana_chain_spec::ChainSpec;
    use katana_executor::blockifier::cache::ClassCacheBuilder;
    use katana_executor::ExecutionFlags;
    use katana_pool_api::validation::{ValidationOutcome, Validator};
    use katana_primitives::block::{Block, FinalityStatus, SealedBlockWithStatus};
    use katana_primitives::chain::ChainId;
    use katana_primitives::contract::{ContractAddress, Nonce};
    use katana_primitives::env::BlockEnv;
    use katana_primitives::transaction::{
        ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV1,
    };
    use katana_primitives::Felt;
    use katana_provider::api::block::BlockWriter;
    use katana_provider::api::state::{StateFactoryProvider, StateProvider};
    use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
    use parking_lot::Mutex;

    use super::TxValidator;

    fn create_test_state(chain_spec: &ChainSpec) -> Box<dyn StateProvider> {
        let ChainSpec::Dev(chain) = chain_spec else { panic!("should be dev chain spec") };
        let states = chain.state_updates();
        let provider_factory = DbProviderFactory::new_in_memory();
        let provider_mut = provider_factory.provider_mut();
        let block = SealedBlockWithStatus {
            status: FinalityStatus::AcceptedOnL2,
            block: Block::default().seal_with_hash(Felt::ZERO),
        };
        provider_mut.insert_block_with_states_and_receipts(block, states, vec![], vec![]).unwrap();
        provider_mut.commit().unwrap();
        provider_factory.provider().latest().unwrap()
    }

    fn create_invoke_tx(
        sender: ContractAddress,
        chain_id: ChainId,
        nonce: Nonce,
    ) -> ExecutableTxWithHash {
        ExecutableTxWithHash::new(ExecutableTx::Invoke(InvokeTx::V1(InvokeTxV1 {
            chain_id,
            sender_address: sender,
            nonce,
            calldata: vec![],
            signature: vec![],
            max_fee: 1_000_000_000_000_000,
        })))
    }

    /// Reproduces the pool_nonces drift bug after `validator.update()`.
    ///
    /// `pool_nonces` tracks the expected next nonce per account based on validated
    /// transactions. When `update()` is called after a block is mined, it replaces
    /// the state and block env but does NOT clear `pool_nonces`. If none of the
    /// validated transactions were actually committed (e.g. they were dropped or
    /// the block was empty), pool_nonces retains stale values that are far ahead
    /// of the actual state nonce.
    ///
    /// This causes the validator to skip the `tx_nonce > current_nonce` check
    /// (which would correctly flag the tx as Dependent) and fall through to the
    /// blockifier, which — with `strict_nonce_check = false` — allows any
    /// `account_nonce <= tx_nonce`. Since the real state nonce is 0, a tx with
    /// nonce 3 passes validation despite the massive nonce gap.
    ///
    /// In production, this manifests as:
    ///   "Invalid transaction nonce of contract at address ... Account nonce: 0xN; got: 0xM."
    /// where M >> N, because the executor sees the real state nonce.
    ///
    /// This test FAILS with the current buggy code and PASSES once the fix
    /// (clearing pool_nonces in update()) is applied.
    #[tokio::test]
    async fn pool_nonces_must_be_cleared_after_validator_update() {
        // Initialize the global class cache (required by the blockifier)
        let _ = ClassCacheBuilder::new().build_global();

        let chain_spec = Arc::new(ChainSpec::dev());
        let chain_id = chain_spec.id();
        let sender = *chain_spec.genesis().accounts().next().unwrap().0;

        let state = create_test_state(&chain_spec);
        let execution_flags = ExecutionFlags::new().with_account_validation(false).with_fee(false);
        let block_env = BlockEnv::default();
        let permit = Arc::new(Mutex::new(()));

        let validator = TxValidator::new(
            state,
            execution_flags,
            None,
            block_env.clone(),
            permit,
            chain_spec.clone(),
        );

        // Validate 3 txs with nonces 0, 1, 2 — all should pass as Valid.
        // This advances pool_nonces[sender] to 3.
        for nonce in 0..3u64 {
            let tx = create_invoke_tx(sender, chain_id, Felt::from(nonce));
            let result = validator.validate(tx).await;
            assert!(
                matches!(result, Ok(ValidationOutcome::Valid(_))),
                "tx with nonce {nonce} should be Valid"
            );
        }

        // Simulate block production where NONE of the txs were committed
        // (e.g. they were dropped, reverted, or the block was empty).
        // update() replaces state (nonce = 0) but pool_nonces stays at 3.
        let fresh_state = create_test_state(&chain_spec);
        validator.update(fresh_state, block_env);

        // Now validate a tx with nonce 3. With the bug:
        //   - current_nonce = pool_nonces[sender] = 3 (stale, not cleared)
        //   - tx_nonce(3) == current_nonce(3) → falls through to blockifier
        //   - blockifier (non-strict): state_nonce(0) <= tx_nonce(3) → passes
        //   - Result: Valid ← WRONG! nonce gap of 3 from actual state
        //
        // After fix (pool_nonces cleared in update()):
        //   - current_nonce = state.nonce(sender) = 0
        //   - tx_nonce(3) > current_nonce(0) → Dependent ← CORRECT
        let tx = create_invoke_tx(sender, chain_id, Felt::THREE);
        let result = validator.validate(tx).await;

        assert!(
            matches!(result, Ok(ValidationOutcome::Dependent { .. })),
            "After update(), tx with nonce 3 should be Dependent (state nonce is 0), but stale \
             pool_nonces caused it to be accepted as Valid. Got: {result:?}"
        );
    }

    /// End-to-end test reproducing the production error through the full
    /// pool → executor pipeline, mirroring the actual node setup.
    ///
    /// When pool_nonces drifts ahead of the actual state nonce (the bug this
    /// fix addresses), a transaction whose nonce matches the drifted value
    /// passes the pool validator's checks and enters the pool. The block
    /// producer then picks it up from the pool and feeds it to the executor.
    /// The executor uses strict nonce checking and rejects it with:
    ///
    ///   "Invalid transaction nonce of contract at address 0x...
    ///    Account nonce: 0x0; got: 0x3."
    ///
    /// This is the same error observed in production:
    ///   "Invalid transaction nonce of contract at address 0x4250...bccf.
    ///    Account nonce: 0x27ed; got: 0x2bbf."
    ///
    /// The test exercises the full flow:
    ///   1. Tx submitted to pool via `add_transaction` (goes through TxValidator)
    ///   2. Tx picked up from pool via `pending_transactions` (like block producer)
    ///   3. Tx executed by blockifier executor
    ///   4. Executor rejects with InvalidNonce
    #[tokio::test]
    async fn pool_to_executor_nonce_drift_produces_invalid_nonce_error() {
        use futures::StreamExt;
        use katana_executor::blockifier::BlockifierFactory;
        use katana_executor::{ExecutionResult, ExecutorFactory};
        use katana_pool_api::TransactionPool;
        use katana_primitives::env::VersionedConstantsOverrides;

        use crate::ordering::FiFo;
        use crate::pool::Pool;

        let _ = ClassCacheBuilder::new().build_global();

        let chain_spec = Arc::new(ChainSpec::dev());
        let chain_id = chain_spec.id();
        let sender = *chain_spec.genesis().accounts().next().unwrap().0;

        // -- Set up pool with TxValidator (same as real node) --
        let state = create_test_state(&chain_spec);
        let execution_flags = ExecutionFlags::new().with_account_validation(false).with_fee(false);
        let block_env = BlockEnv::default();
        let permit = Arc::new(Mutex::new(()));

        let validator = TxValidator::new(
            state,
            execution_flags.clone(),
            None,
            block_env.clone(),
            permit,
            chain_spec.clone(),
        );

        let pool = Pool::new(validator.clone(), FiFo::new());

        // -- Set up executor factory (same config as block producer) --
        let executor_factory = BlockifierFactory::new(
            Some(VersionedConstantsOverrides {
                validate_max_n_steps: Some(u32::MAX),
                invoke_tx_max_n_steps: Some(u32::MAX),
                max_recursion_depth: Some(usize::MAX),
            }),
            // Executor uses default flags: strict nonce check enabled
            execution_flags,
            katana_executor::BlockLimits::default(),
            katana_executor::blockifier::cache::ClassCache::global(),
            chain_spec.clone(),
        );

        // -- Simulate pool_nonces drift (pre-fix bug) --
        //
        // In production, pool_nonces drifts because update() didn't clear it.
        // After many blocks, pool_nonces[sender] could be far ahead of state.
        // We simulate this by directly setting pool_nonces to a drifted value.
        let drifted_nonce = Felt::THREE;
        {
            let mut inner = validator.inner.lock();
            inner.pool_nonces.insert(sender, drifted_nonce);
        }

        // -- Step 1: Submit tx to pool via add_transaction --
        //
        // The validator sees pool_nonces[sender] = 3, tx_nonce = 3.
        // Since tx_nonce == current_nonce, it falls through to blockifier.
        // Blockifier (non-strict in validator): state_nonce(0) <= tx_nonce(3) → passes.
        // Tx enters the pool as Valid.
        let tx = create_invoke_tx(sender, chain_id, drifted_nonce);
        let tx_hash = tx.hash;
        pool.add_transaction(tx).await.expect("tx should pass validation and enter pool");

        assert!(pool.contains(tx_hash), "tx should be in the pool");

        // -- Step 2: Pull tx from pool (like block producer does) --
        let mut pending = pool.pending_transactions();
        let pending_tx = pending.next().await.expect("should have a pending tx");
        let picked_tx = pending_tx.tx.as_ref().clone();
        assert_eq!(picked_tx.hash, tx_hash);

        // -- Step 3: Execute with executor (like block producer does) --
        //
        // Executor is created with fresh state (nonce = 0) and strict nonce check.
        // This is what happens in the real node: the executor's state reflects
        // committed blocks, not the pool's speculative nonce tracking.
        let executor_state = create_test_state(&chain_spec);
        let mut executor = executor_factory.executor(executor_state, block_env);

        let (executed, _) = executor.execute_transactions(vec![picked_tx]).unwrap();
        assert_eq!(executed, 1, "executor should process the tx (even if it fails)");

        // -- Step 4: Verify the exact production error --
        let (_, result) = &executor.transactions()[0];
        match result {
            ExecutionResult::Failed { error } => {
                let error_msg = error.to_string();
                // This is the exact error format from production logs:
                //   "Invalid transaction nonce of contract at address {addr}.
                //    Account nonce: {current_nonce:#x}; got: {tx_nonce:#x}."
                assert!(
                    error_msg.contains("Invalid transaction nonce"),
                    "Expected InvalidNonce error, got: {error_msg}"
                );
                assert!(
                    error_msg.contains(&format!("{sender}")),
                    "Error should reference the sender address, got: {error_msg}"
                );
                assert!(
                    error_msg.contains("Account nonce: 0x0"),
                    "Error should show state nonce 0x0, got: {error_msg}"
                );
                assert!(
                    error_msg.contains("got: 0x3"),
                    "Error should show tx nonce 0x3, got: {error_msg}"
                );
            }
            ExecutionResult::Success { .. } => {
                panic!(
                    "Executor should reject tx with nonce 3 when state nonce is 0, but it \
                     succeeded"
                );
            }
        }

        // -- Cleanup: remove from pool (like block producer does post-execution) --
        pool.remove_transactions(&[tx_hash]);
        assert!(!pool.contains(tx_hash));
    }
}
