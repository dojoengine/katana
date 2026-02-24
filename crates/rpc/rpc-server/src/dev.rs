use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_core::backend::Backend;
use katana_core::service::block_producer::{BlockProducer, PendingExecutor};
use katana_primitives::contract::{ContractAddress, StorageKey, StorageValue};
use katana_provider::api::state::StateWriter;
use katana_provider::{MutableProvider, ProviderFactory, ProviderRO, ProviderRW};
use katana_rpc_api::dev::DevApiServer;
use katana_rpc_api::error::dev::DevApiError;
use katana_rpc_types::account::Account;

#[allow(missing_debug_implementations)]
pub struct DevApi<PF>
where
    PF: ProviderFactory,
{
    backend: Arc<Backend<PF>>,
    block_producer: BlockProducer<PF>,
}

impl<PF> DevApi<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    pub fn new(backend: Arc<Backend<PF>>, block_producer: BlockProducer<PF>) -> Self {
        Self { backend, block_producer }
    }

    /// Returns the pending executor snapshot managed by the block producer.
    fn pending_executor(&self) -> Option<PendingExecutor> {
        self.block_producer.pending_executor()
    }

    fn has_pending_transactions(&self) -> bool {
        self.block_producer.has_pending_transactions()
    }

    pub fn set_next_block_timestamp(&self, timestamp: u64) -> Result<(), DevApiError> {
        if self.has_pending_transactions() {
            return Err(DevApiError::PendingTransactions);
        }

        let mut block_context_generator = self.backend.block_context_generator.write();
        block_context_generator.next_block_start_time = timestamp;

        Ok(())
    }

    pub fn increase_next_block_timestamp(&self, offset: u64) -> Result<(), DevApiError> {
        if self.has_pending_transactions() {
            return Err(DevApiError::PendingTransactions);
        }

        let mut block_context_generator = self.backend.block_context_generator.write();
        block_context_generator.block_timestamp_offset += offset as i64;

        Ok(())
    }

    pub fn set_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        value: StorageValue,
    ) -> Result<(), DevApiError> {
        // If there's a pending executor, update the pending state so the change is visible to
        // the next block view immediately.
        if let Some(pending_executor) = self.pending_executor() {
            // Leaky-leaky abstraction:
            // The logic here might seem counterintuitive because we're taking a non-mutable
            // reference (ie read lock) but we're allowed to update the pending state.
            pending_executor
                .read()
                .set_storage_at(contract_address, key, value)
                .map_err(DevApiError::unexpected_error)?;
        } else {
            let provider = self.backend.storage.provider_mut();

            provider
                .set_storage(contract_address, key, value)
                .map_err(DevApiError::unexpected_error)?;

            provider.commit().map_err(DevApiError::unexpected_error)?;
        }

        Ok(())
    }
}

#[async_trait]
impl<PF> DevApiServer for DevApi<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    async fn generate_block(&self) -> RpcResult<()> {
        self.block_producer.force_mine();
        Ok(())
    }

    async fn next_block_timestamp(&self) -> RpcResult<()> {
        // Ok(self.sequencer.backend().env.read().block.block_timestamp.0)
        Ok(())
    }

    async fn set_next_block_timestamp(&self, timestamp: u64) -> RpcResult<()> {
        Ok(self.set_next_block_timestamp(timestamp)?)
    }

    async fn increase_next_block_timestamp(&self, timestamp: u64) -> RpcResult<()> {
        Ok(self.increase_next_block_timestamp(timestamp)?)
    }

    async fn set_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        value: StorageValue,
    ) -> RpcResult<()> {
        Ok(self.set_storage_at(contract_address, key, value)?)
    }

    async fn predeployed_accounts(&self) -> RpcResult<Vec<Account>> {
        Ok(self.backend.chain_spec.genesis().accounts().map(|e| Account::new(*e.0, e.1)).collect())
    }
}
