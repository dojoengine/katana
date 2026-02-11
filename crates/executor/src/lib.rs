#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod utils;

pub mod error;

pub mod blockifier;
pub mod noop;

use katana_primitives::block::ExecutableBlock;
use katana_primitives::contract::{ContractAddress, StorageKey, StorageValue};
use katana_primitives::env::{BlockEnv, VersionedConstantsOverrides};
use katana_primitives::execution::TransactionExecutionInfo;
use katana_primitives::receipt::Receipt;
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::transaction::{ExecutableTxWithHash, TxWithHash};
use katana_provider::api::state::StateProvider;

pub type ExecutorResult<T> = Result<T, error::ExecutorError>;

/// See <https://docs.starknet.io/chain-info/#current_limits>.
#[derive(Debug, Clone)]
pub struct BlockLimits {
    /// The maximum number of Cairo steps that can be completed within each block.
    pub cairo_steps: u64,
}

impl Default for BlockLimits {
    fn default() -> Self {
        Self { cairo_steps: 50_000_000 }
    }
}

/// Transaction execution simulation flags.
///
/// These flags can be used to control the behavior of the transaction execution, such as skipping
/// the transaction validation, or ignoring any fee related checks.
#[derive(Debug, Clone)]
pub struct ExecutionFlags {
    /// Determine whether to perform the transaction sender's account validation logic.
    account_validation: bool,
    /// Determine whether to perform fee related checks and operations ie., fee transfer.
    fee: bool,
    /// Determine whether to perform transaction's sender nonce check.
    nonce_check: bool,
}

impl Default for ExecutionFlags {
    fn default() -> Self {
        Self { account_validation: true, fee: true, nonce_check: true }
    }
}

impl ExecutionFlags {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether to enable or disable the account validation.
    pub fn with_account_validation(mut self, enable: bool) -> Self {
        self.account_validation = enable;
        self
    }

    /// Set whether to enable or disable the fee related operations.
    pub fn with_fee(mut self, enable: bool) -> Self {
        self.fee = enable;
        self
    }

    /// Set whether to enable or disable the nonce check.
    pub fn with_nonce_check(mut self, enable: bool) -> Self {
        self.nonce_check = enable;
        self
    }

    /// Returns whether the account validation is enabled.
    pub fn account_validation(&self) -> bool {
        self.account_validation
    }

    /// Returns whether the fee related operations are enabled.
    pub fn fee(&self) -> bool {
        self.fee
    }

    /// Returns whether the nonce check is enabled.
    pub fn nonce_check(&self) -> bool {
        self.nonce_check
    }
}

/// Stats about the transactions execution.
#[derive(Debug, Clone, Default)]
pub struct ExecutionStats {
    /// The total gas used.
    pub l1_gas_used: u128,
    /// The total cairo steps used.
    pub cairo_steps_used: u128,
}

/// The output of a executor after a series of executions.
#[derive(Debug, Default)]
pub struct ExecutionOutput {
    /// Statistics throughout the executions process.
    pub stats: ExecutionStats,
    /// The state updates produced by the executions.
    pub states: StateUpdatesWithClasses,
    /// The transactions that have been executed.
    pub transactions: Vec<(TxWithHash, ExecutionResult)>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum ExecutionResult {
    Success { receipt: Receipt, trace: TransactionExecutionInfo },
    Failed { error: error::ExecutionError },
}

impl ExecutionResult {
    /// Creates a new successful execution result.
    pub fn new_success(receipt: Receipt, trace: TransactionExecutionInfo) -> Self {
        ExecutionResult::Success { receipt, trace }
    }

    /// Creates a new failed execution result with the given error.
    pub fn new_failed(error: impl Into<error::ExecutionError>) -> Self {
        ExecutionResult::Failed { error: error.into() }
    }

    /// Returns `true` if the execution was successful.
    pub fn is_success(&self) -> bool {
        matches!(self, ExecutionResult::Success { .. })
    }

    /// Returns `true` if the execution failed.
    pub fn is_failed(&self) -> bool {
        !self.is_success()
    }

    /// Returns the receipt of the execution if it was successful. Otherwise, returns `None`.
    pub fn receipt(&self) -> Option<&Receipt> {
        match self {
            ExecutionResult::Success { receipt, .. } => Some(receipt),
            _ => None,
        }
    }

    /// Returns the execution info if it was successful. Otherwise, returns `None`.
    pub fn trace(&self) -> Option<&TransactionExecutionInfo> {
        match self {
            ExecutionResult::Success { trace, .. } => Some(trace),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResultAndStates {
    pub result: ExecutionResult,
    pub states: StateUpdates,
}

/// A type that can create [BlockExecutor] instance.
pub trait ExecutorFactory: Send + Sync + 'static + core::fmt::Debug {
    /// Create a [Executor] for executing transactions.
    fn executor(&self, state: Box<dyn StateProvider>, block_env: BlockEnv) -> Box<dyn Executor>;

    fn overrides(&self) -> Option<&VersionedConstantsOverrides>;

    /// Returns the execution flags set by the factory.
    fn execution_flags(&self) -> &ExecutionFlags;
}

/// An executor that can execute a block of transactions.
pub trait Executor: Send + Sync + core::fmt::Debug {
    /// Executes the given block.
    fn execute_block(&mut self, block: ExecutableBlock) -> ExecutorResult<()>;

    /// Execute transactions and returns the total number of transactions that was executed.
    fn execute_transactions(
        &mut self,
        transactions: Vec<ExecutableTxWithHash>,
    ) -> ExecutorResult<(usize, Option<error::ExecutorError>)>;

    /// Takes the output state of the executor.
    fn take_execution_output(&mut self) -> ExecutorResult<ExecutionOutput>;

    /// Returns the current state of the executor.
    fn state(&self) -> Box<dyn StateProvider>;

    /// Returns the transactions that have been executed.
    fn transactions(&self) -> &[(TxWithHash, ExecutionResult)];

    /// Returns the current block environment of the executor.
    fn block_env(&self) -> BlockEnv;

    // TEMP: This is primarily for `dev_setStorageAt` dev endpoint. To make sure the updated storage
    // value is reflected in the pending state. This functionality should prolly be moved to the
    // pending state level instead of the executor.
    //
    /// Sets the storage value for the given contract address and key.
    /// This is used for dev purposes to manipulate state directly.
    fn set_storage_at(
        &self,
        _address: ContractAddress,
        _key: StorageKey,
        _value: StorageValue,
    ) -> ExecutorResult<()> {
        Ok(()) // default no-op
    }
}
