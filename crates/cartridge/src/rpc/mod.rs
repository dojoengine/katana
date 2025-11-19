use std::future::Future;
use std::sync::Arc;

use anyhow::anyhow;
use cainome::cairo_serde::CairoSerde;
use jsonrpsee::core::{async_trait, RpcResult};
use katana_core::backend::Backend;
use katana_core::service::block_producer::{BlockProducer, BlockProducerMode};
use katana_executor::ExecutorFactory;
use katana_pool::{TransactionPool, TxPool};
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::broadcasted::AddInvokeTransactionResponse;
use katana_rpc_types::FunctionCall;
use katana_tasks::{Result as TaskResult, TaskSpawner};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tracing::debug;
use types::OutsideExecution;

mod api;
pub mod types;

pub use api::*;

use crate::utils::encode_calls;

#[allow(missing_debug_implementations)]
pub struct CartridgeApi<EF: ExecutorFactory> {
    task_spawner: TaskSpawner,
    block_producer: BlockProducer<EF>,
    backend: Arc<Backend<EF>>,
    pool: TxPool,
}

impl<EF: ExecutorFactory> CartridgeApi<EF> {
    pub fn new(
        backend: Arc<Backend<EF>>,
        block_producer: BlockProducer<EF>,
        pool: TxPool,
        task_spawner: TaskSpawner,
    ) -> Self {
        Self { backend, block_producer, pool, task_spawner }
    }

    fn nonce(&self, address: ContractAddress) -> Result<Option<Nonce>, StarknetApiError> {
        match self.pool.get_nonce(address) {
            pending_nonce @ Some(..) => Ok(pending_nonce),
            None => Ok(self.state()?.nonce(address)?),
        }
    }

    fn state(&self) -> Result<Box<dyn StateProvider>, StarknetApiError> {
        match &*self.block_producer.producer.read() {
            BlockProducerMode::Instant(_) => Ok(self.backend.blockchain.provider().latest()?),
            BlockProducerMode::Interval(producer) => Ok(producer.executor().read().state()),
        }
    }

    pub async fn execute_outside(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
    ) -> Result<AddInvokeTransactionResponse, StarknetApiError> {
        debug!(%address, ?outside_execution, "Adding execute outside transaction.");
        self.on_cpu_blocking_task(move |this| async move {
            let (pm_address, pm_account) = this
                .backend
                .chain_spec
                .genesis()
                .paymaster_account()
                .ok_or(anyhow!("Cartridge paymaster account doesn't exist"))?;

            // Contract function selector for
            let entrypoint = match outside_execution {
                OutsideExecution::V2(_) => selector!("execute_from_outside_v2"),
                OutsideExecution::V3(_) => selector!("execute_from_outside_v3"),
            };

            // Get the current nonce of the paymaster account.
            let nonce = this.nonce(pm_address)?.unwrap_or_default();

            let mut inner_calldata = OutsideExecution::cairo_serialize(&outside_execution);
            inner_calldata.extend(Vec::<Felt>::cairo_serialize(&signature));

            let execute_from_outside_call = FunctionCall {
                contract_address: address,
                entry_point_selector: entrypoint,
                calldata: inner_calldata,
            };

            let chain_id = this.backend.chain_spec.id();

            let mut tx = InvokeTxV3 {
                nonce,
                chain_id,
                calldata: encode_calls(vec![execute_from_outside_call]),
                signature: vec![],
                sender_address: pm_address,
                tip: 0_u64,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                fee_data_availability_mode: DataAvailabilityMode::L1,
                resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
            };
            let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

            let signer = LocalWallet::from(SigningKey::from_secret_scalar(pm_account.private_key));
            let signature =
                futures::executor::block_on(signer.sign_hash(&tx_hash)).map_err(|e| anyhow!(e))?;
            tx.signature = vec![signature.r, signature.s];

            let tx = ExecutableTxWithHash::new(ExecutableTx::Invoke(InvokeTx::V3(tx)));
            let transaction_hash = this.pool.add_transaction(tx).await?;

            Ok(AddInvokeTransactionResponse { transaction_hash })
        })
        .await?
    }

    /// Spawns an async function that is mostly CPU-bound blocking task onto the manager's blocking
    /// pool.
    async fn on_cpu_blocking_task<T, F>(&self, func: T) -> Result<F::Output, StarknetApiError>
    where
        T: FnOnce(Self) -> F,
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        use tokio::runtime::Builder;

        let this = self.clone();
        let future = func(this);
        let span = tracing::Span::current();

        let task = move || {
            let _enter = span.enter();
            Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime")
                .block_on(future)
        };

        match self.task_spawner.cpu_bound().spawn(task).await {
            TaskResult::Ok(result) => Ok(result),
            TaskResult::Err(err) => {
                Err(StarknetApiError::unexpected(format!("internal task execution failed: {err}")))
            }
        }
    }
}

impl<EF: ExecutorFactory> Clone for CartridgeApi<EF> {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            block_producer: self.block_producer.clone(),
            backend: self.backend.clone(),
            task_spawner: self.task_spawner.clone(),
        }
    }
}

#[async_trait]
impl<EF: ExecutorFactory> CartridgeApiServer for CartridgeApi<EF> {
    async fn add_execute_outside_transaction(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        Ok(self.execute_outside(address, outside_execution, signature).await?)
    }
}
