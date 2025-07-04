use katana_primitives::block::ExecutableBlock;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::env::{BlockEnv, CfgEnv};
use katana_primitives::transaction::{ExecutableTxWithHash, TxWithHash};
use katana_provider::traits::contract::ContractClassProvider;
use katana_provider::traits::state::{StateProofProvider, StateProvider, StateRootProvider};
use katana_provider::ProviderResult;

use crate::abstraction::{
    BlockExecutor, ExecutionFlags, ExecutionOutput, ExecutionResult, ExecutorFactory,
    ExecutorResult,
};
use crate::ExecutorError;

/// A no-op executor factory. Creates an executor that does nothing.
#[derive(Debug, Default)]
pub struct NoopExecutorFactory {
    cfg: CfgEnv,
    execution_flags: ExecutionFlags,
}

impl NoopExecutorFactory {
    /// Create a new no-op executor factory.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ExecutorFactory for NoopExecutorFactory {
    fn with_state<'a, P>(&self, state: P) -> Box<dyn BlockExecutor<'a> + 'a>
    where
        P: StateProvider + 'a,
    {
        let _ = state;
        Box::<NoopExecutor>::default()
    }

    fn with_state_and_block_env<'a, P>(
        &self,
        state: P,
        block_env: BlockEnv,
    ) -> Box<dyn BlockExecutor<'a> + 'a>
    where
        P: StateProvider + 'a,
    {
        let _ = state;
        let _ = block_env;
        Box::new(NoopExecutor { block_env })
    }

    fn cfg(&self) -> &CfgEnv {
        &self.cfg
    }

    fn execution_flags(&self) -> &ExecutionFlags {
        &self.execution_flags
    }
}

#[derive(Debug, Default)]
struct NoopExecutor {
    block_env: BlockEnv,
}

impl<'a> BlockExecutor<'a> for NoopExecutor {
    fn execute_block(&mut self, block: ExecutableBlock) -> ExecutorResult<()> {
        let _ = block;
        Ok(())
    }

    fn execute_transactions(
        &mut self,
        transactions: Vec<ExecutableTxWithHash>,
    ) -> ExecutorResult<(usize, Option<ExecutorError>)> {
        Ok((transactions.len(), None))
    }

    fn take_execution_output(&mut self) -> ExecutorResult<ExecutionOutput> {
        Ok(ExecutionOutput::default())
    }

    fn state(&self) -> Box<dyn StateProvider + 'a> {
        Box::new(NoopStateProvider)
    }

    fn transactions(&self) -> &[(TxWithHash, ExecutionResult)] {
        &[]
    }

    fn block_env(&self) -> BlockEnv {
        self.block_env.clone()
    }
}

#[derive(Debug)]
struct NoopStateProvider;

impl ContractClassProvider for NoopStateProvider {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        let _ = hash;
        Ok(None)
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        let _ = hash;
        Ok(None)
    }
}

impl StateProvider for NoopStateProvider {
    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        let _ = address;
        Ok(None)
    }

    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        let _ = address;
        Ok(None)
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        let _ = address;
        let _ = storage_key;
        Ok(None)
    }
}

impl StateProofProvider for NoopStateProvider {}
impl StateRootProvider for NoopStateProvider {}
