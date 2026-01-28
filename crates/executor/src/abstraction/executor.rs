use katana_primitives::block::ExecutableBlock;
use katana_primitives::contract::{ContractAddress, StorageKey, StorageValue};
use katana_primitives::env::{BlockEnv, CfgEnv};
use katana_primitives::transaction::{ExecutableTxWithHash, TxWithHash};
use katana_provider::api::state::StateProvider;

use super::ExecutorError;
use crate::{ExecutionFlags, ExecutionOutput, ExecutionResult, ExecutorResult};

/// A type that can create [BlockExecutor] instance.
pub trait ExecutorFactory: Send + Sync + 'static + core::fmt::Debug {
    /// Construct a new [BlockExecutor] with the given state.
    fn with_state<'a, P>(&self, state: P) -> Box<dyn BlockExecutor<'a> + 'a>
    where
        P: StateProvider + 'a;

    /// Construct a new [BlockExecutor] with the given state and block environment values.
    fn with_state_and_block_env<'a, P>(
        &self,
        state: P,
        block_env: BlockEnv,
    ) -> Box<dyn BlockExecutor<'a> + 'a>
    where
        P: StateProvider + 'a;

    /// Returns the configuration environment of the factory.
    fn cfg(&self) -> &CfgEnv;

    /// Returns the execution flags set by the factory.
    fn execution_flags(&self) -> &ExecutionFlags;
}

/// An executor that can execute a block of transactions.
pub trait BlockExecutor<'a>: Send + Sync + core::fmt::Debug {
    /// Executes the given block.
    fn execute_block(&mut self, block: ExecutableBlock) -> ExecutorResult<()>;

    /// Execute transactions and returns the total number of transactions that was executed.
    fn execute_transactions(
        &mut self,
        transactions: Vec<ExecutableTxWithHash>,
    ) -> ExecutorResult<(usize, Option<ExecutorError>)>;

    /// Takes the output state of the executor.
    fn take_execution_output(&mut self) -> ExecutorResult<ExecutionOutput>;

    /// Returns the current state of the executor.
    fn state(&self) -> Box<dyn StateProvider + 'a>;

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
        address: ContractAddress,
        key: StorageKey,
        value: StorageValue,
    ) -> ExecutorResult<()>;
}
