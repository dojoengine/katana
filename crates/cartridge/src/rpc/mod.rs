//! Handles management of Cartridge controller accounts.
//!
//! When a Controller account is created, the username is used as a salt,
//! and the latest controller class hash is used.
//! This ensures that the controller account address is deterministic.
//!
//! A consequence of that, is that all the controller class hashes must be
//! known by Katana to ensure it can first deploy the controller account with the
//! correct address, and then upgrade it to the latest version.
//!
//! This module contains the function to work around this behavior, which also relies
//! on the updated code into `katana-primitives` to ensure all the controller class hashes
//! are available.
//!
//! Two flows:
//!
//! 1. When a Controller account is created, an execution from outside is received from the very
//!    first transaction that the user will want to achieve using the session. In this case, this
//!    module will hook the execution from outside to ensure the controller account is deployed.
//!
//! 2. When a Controller account is already deployed, and the user logs in, the client code of
//!    controller is actually performing a `estimate_fee` to estimate the fee for the account
//!    upgrade. In this case, this module contains the code to hook the fee estimation, and return
//!    the associated transaction to be executed in order to deploy the controller account. See the
//!    fee estimate RPC method of [StarknetApi](crate::starknet::StarknetApi) to see how the
//!    Controller deployment is handled during fee estimation.

use std::future::Future;
use std::sync::Arc;

use anyhow::anyhow;
use cainome::cairo_serde::CairoSerde;
use jsonrpsee::core::{async_trait, RpcResult};
use katana_core::backend::Backend;
use katana_core::service::block_producer::{BlockProducer, BlockProducerMode};
use katana_executor::ExecutorFactory;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_pool::{TransactionPool, TxPool};
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::genesis::allocation::GenesisAccountAlloc;
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::traits::state::{StateFactoryProvider, StateProvider};
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::broadcasted::AddInvokeTransactionResponse;
use katana_rpc_types::outside_execution::{
    OutsideExecution, OutsideExecutionV2, OutsideExecutionV3,
};
use katana_rpc_types::transaction::InvokeTxResult;
use katana_rpc_types::FunctionCall;
use katana_tasks::TokioTaskSpawner;
use katana_tasks::{Result as TaskResult, TaskSpawner};
use starknet::core::types::Call;
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use starknet_crypto::pedersen_hash;
use tracing::{debug, info};
use types::OutsideExecution;

mod api;
pub mod types;

pub use api::*;

use crate::utils::encode_calls;

#[allow(missing_debug_implementations)]
pub struct CartridgeApi<EF: ExecutorFactory> {
    task_spawner: TaskSpawner,
    backend: Arc<Backend<EF>>,
    pool: TxPool,
}

impl<EF: ExecutorFactory> CartridgeApi<EF> {
    pub fn new(backend: Arc<Backend<EF>>, pool: TxPool) -> Self {
        Self { backend, pool }
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
            // For now, we use the first predeployed account in the genesis as the paymaster
            // account.
            let (pm_address, pm_acc) = this
                .backend
                .chain_spec
                .genesis()
                .accounts()
                .nth(0)
                .ok_or(anyhow!("Cartridge paymaster account doesn't exist"))?;

            // TODO: create a dedicated types for aux accounts (eg paymaster)
            let pm_private_key = if let GenesisAccountAlloc::DevAccount(pm) = pm_acc {
                pm.private_key
            } else {
                return Err(StarknetApiError::unexpected("Paymaster is not a dev account"));
            };

            // Contract function selector for
            let entrypoint = match outside_execution {
                OutsideExecution::V2(_) => selector!("execute_from_outside_v2"),
                OutsideExecution::V3(_) => selector!("execute_from_outside_v3"),
            };

            // Get the current nonce of the paymaster account.
            let nonce = this.nonce(*pm_address)?.unwrap_or_default();

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
                sender_address: *pm_address,
                tip: 0_u64,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                fee_data_availability_mode: DataAvailabilityMode::L1,
                resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
            };
            let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

            let signer = LocalWallet::from(SigningKey::from_secret_scalar(pm_private_key));
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
        Self { pool: self.pool.clone(), backend: self.backend.clone() }
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
/// Encodes the given calls into a vector of Felt values (New encoding, cairo 1),
/// since controller accounts are Cairo 1 contracts.
pub fn encode_calls(calls: Vec<FunctionCall>) -> Vec<Felt> {
    let mut execute_calldata: Vec<Felt> = vec![calls.len().into()];
    for call in calls {
        execute_calldata.push(call.contract_address.into());
        execute_calldata.push(call.entry_point_selector);

        execute_calldata.push(call.calldata.len().into());
        execute_calldata.extend_from_slice(&call.calldata);
    }

    execute_calldata
}
