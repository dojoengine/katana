use katana_primitives::block::ExecutableBlock;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::env::{BlockEnv, VersionedConstantsOverrides};
use katana_primitives::transaction::{ExecutableTxWithHash, TxWithHash};
use katana_provider::api::contract::ContractClassProvider;
use katana_provider::api::state::{StateProofProvider, StateProvider, StateRootProvider};
use katana_provider::api::ProviderResult;

use crate::abstraction::{
    ExecutionFlags, ExecutionOutput, ExecutionResult, Executor, ExecutorFactory, ExecutorResult,
};
use crate::ExecutorError;

/// A no-op executor factory. Creates an executor that does nothing.
#[derive(Debug, Default)]
pub struct NoopExecutorFactory {
    execution_flags: ExecutionFlags,
}

impl NoopExecutorFactory {
    /// Create a new no-op executor factory.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ExecutorFactory for NoopExecutorFactory {
    fn executor(&self, _state: Box<dyn StateProvider>, block_env: BlockEnv) -> Box<dyn Executor> {
        Box::new(NoopExecutor { block_env })
    }

    fn overrides(&self) -> Option<&VersionedConstantsOverrides> {
        None
    }

    fn execution_flags(&self) -> &ExecutionFlags {
        &self.execution_flags
    }
}

#[derive(Debug, Default)]
struct NoopExecutor {
    block_env: BlockEnv,
}

impl Executor for NoopExecutor {
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

    fn state(&self) -> Box<dyn StateProvider> {
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
