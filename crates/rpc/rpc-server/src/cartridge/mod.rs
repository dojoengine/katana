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

mod vrf;

use std::future::Future;
use std::sync::Arc;

use anyhow::anyhow;
use cainome::cairo_serde::CairoSerde;
use http::{HeaderMap, HeaderName, HeaderValue};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use katana_core::backend::Backend;
use katana_core::service::block_producer::{BlockProducer, BlockProducerMode};
use katana_executor::ExecutorFactory;
use katana_genesis::constant::{DEFAULT_STRK_FEE_TOKEN_ADDRESS, DEFAULT_UDC_ADDRESS};
use katana_pool::{TransactionPool, TxPool};
use katana_primitives::chain::ChainId;
use katana_primitives::contract::Nonce;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::paymaster::PaymasterApiClient;
use katana_rpc_types::broadcasted::AddInvokeTransactionResponse;
use katana_rpc_types::cartridge::FeeSource;
use katana_rpc_types::outside_execution::{
    OutsideExecution, OutsideExecutionV2, OutsideExecutionV3,
};
use katana_rpc_types::FunctionCall;
use katana_tasks::{Result as TaskResult, TaskSpawner};
use paymaster_rpc::{
    ExecuteRawRequest, ExecuteRawTransactionParameters, ExecutionParameters, FeeMode,
    RawInvokeParameters,
};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use starknet_paymaster::core::types::Call as PaymasterCall;
use tracing::{debug, info};
use url::Url;
pub use vrf::VrfServiceConfig;
use vrf::{outside_execution_calls_len, request_random_call, VrfService};

#[derive(Debug, Clone)]
pub struct CartridgeConfig {
    pub api_url: Url,
    pub paymaster_url: Url,
    pub paymaster_api_key: Option<String>,
    pub controller_deployer_address: ContractAddress,
    pub controller_deployer_private_key: Felt,
    pub vrf: Option<VrfServiceConfig>,
}

#[allow(missing_debug_implementations)]
pub struct CartridgeApi<EF: ExecutorFactory, PF: ProviderFactory> {
    task_spawner: TaskSpawner,
    backend: Arc<Backend<EF, PF>>,
    block_producer: BlockProducer<EF, PF>,
    pool: TxPool,
    api_client: cartridge::Client,
    paymaster_client: HttpClient,
    /// The paymaster account address used for controller deployment.
    controller_deployer_address: ContractAddress,
    /// The paymaster account private key.
    controller_deployer_private_key: Felt,
    vrf_service: Option<VrfService>,
}

impl<EF, PF> Clone for CartridgeApi<EF, PF>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self {
            task_spawner: self.task_spawner.clone(),
            backend: self.backend.clone(),
            block_producer: self.block_producer.clone(),
            pool: self.pool.clone(),
            api_client: self.api_client.clone(),
            paymaster_client: self.paymaster_client.clone(),
            controller_deployer_address: self.controller_deployer_address,
            controller_deployer_private_key: self.controller_deployer_private_key,
            vrf_service: self.vrf_service.clone(),
        }
    }
}

impl<EF, PF> CartridgeApi<EF, PF>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    pub fn new(
        backend: Arc<Backend<EF, PF>>,
        block_producer: BlockProducer<EF, PF>,
        pool: TxPool,
        task_spawner: TaskSpawner,
        config: CartridgeConfig,
    ) -> anyhow::Result<Self> {
        let api_client = cartridge::Client::new(config.api_url);
        let vrf_service = config.vrf.map(VrfService::new);

        info!(target: "rpc::cartridge", vrf_enabled = vrf_service.is_some(), "Cartridge API initialized.");

        let paymaster_client = {
            let headers = if let Some(api_key) = &config.paymaster_api_key {
                let name = HeaderName::from_static("x-paymaster-api-key");
                let value = HeaderValue::from_str(api_key)?;
                HeaderMap::from_iter([(name, value)])
            } else {
                HeaderMap::default()
            };

            HttpClientBuilder::default().set_headers(headers).build(config.paymaster_url)?
        };

        Ok(Self {
            task_spawner,
            backend,
            block_producer,
            pool,
            api_client,
            paymaster_client,
            controller_deployer_address: config.controller_deployer_address,
            controller_deployer_private_key: config.controller_deployer_private_key,
            vrf_service,
        })
    }

    fn nonce(&self, address: ContractAddress) -> Result<Option<Nonce>, StarknetApiError> {
        match self.pool.get_nonce(address) {
            pending_nonce @ Some(..) => Ok(pending_nonce),
            None => Ok(self.state()?.nonce(address)?),
        }
    }

    fn state(&self) -> Result<Box<dyn StateProvider>, StarknetApiError> {
        match &*self.block_producer.producer.read() {
            BlockProducerMode::Instant(_) => Ok(self.backend.storage.provider().latest()?),
            BlockProducerMode::Interval(producer) => Ok(producer.executor().read().state()),
        }
    }

    pub async fn execute_outside(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
        fee_source: Option<FeeSource>,
    ) -> Result<AddInvokeTransactionResponse, StarknetApiError> {
        debug!(%address, ?outside_execution, "Adding execute outside transaction.");
        self.on_cpu_blocking_task(move |this| async move {
            let pm_address = this.controller_deployer_address;
            let pm_private_key = this.controller_deployer_private_key;

            // ====================== CONTROLLER DEPLOYMENT ======================
            let state = this.state().map(Arc::new)?;
            let is_controller_deployed = state.class_hash_of_contract(address)?.is_some();

            if !is_controller_deployed {
                debug!(target: "rpc::cartridge", controller = %address, "Controller not yet deployed");
                if let Some(tx) = craft_deploy_cartridge_controller_tx(
                    &this.api_client,
                    address,
                    pm_address,
                    pm_private_key,
                    this.backend.chain_spec.id(),
                    this.nonce(pm_address)?.unwrap_or_default(),
                ).await? {
                    debug!(target: "rpc::cartridge", controller = %address, tx = format!("{:#x}", tx.hash), "Inserting Controller deployment transaction");
                    this.pool.add_transaction(tx).await?;
                    this.block_producer.force_mine();
                }
            }
            // ===================================================================

            let mut execute_from_outside_call =
                build_execute_from_outside_call(address, &outside_execution, &signature);
            let mut user_address: Felt = address.into();

            if let Some(vrf_service) = &this.vrf_service {
                if let Some((request_random_call, position)) =
                    request_random_call(&outside_execution)
                {
                    let calls_len = outside_execution_calls_len(&outside_execution);
                    if position + 1 >= calls_len {
                        return Err(StarknetApiError::unexpected(
                            "request_random call must be followed by another call",
                        ));
                    }
                    if request_random_call.to != vrf_service.account_address() {
                        return Err(StarknetApiError::unexpected(
                            "request_random call must target the vrf account",
                        ));
                    }

                    // Delegate VRF computation to the VRF server
                    let chain_id = this.backend.chain_spec.id();
                    let result = vrf_service
                        .outside_execution(address, &outside_execution, &signature, chain_id)
                        .await?;

                    user_address = result.address;
                    execute_from_outside_call =
                        build_execute_from_outside_call_from_vrf_result(&result);
                }
            }

            let fee_mode = match fee_source {
                Some(FeeSource::Credits) => FeeMode::Default {
                    gas_token: DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(),
                    tip: Default::default(),
                },
                _ => FeeMode::Sponsored {
                    tip: Default::default(),
                },
            };

            let request = ExecuteRawRequest {
                transaction: ExecuteRawTransactionParameters::RawInvoke {
                    invoke: RawInvokeParameters {
                        user_address,
                        execute_from_outside_call,
                        gas_token: None,
                        max_gas_token_amount: None,
                    },
                },
                parameters: ExecutionParameters::V1 { fee_mode, time_bounds: None },
            };

            let response = this.paymaster_client.execute_raw_transaction(request).await.map_err(StarknetApiError::unexpected)?;
            Ok(AddInvokeTransactionResponse { transaction_hash: response.transaction_hash })
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

#[async_trait]
impl<EF, PF> CartridgeApiServer for CartridgeApi<EF, PF>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    async fn add_execute_outside_transaction(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
        fee_source: Option<FeeSource>,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        Ok(self.execute_outside(address, outside_execution, signature, fee_source).await?)
    }

    async fn add_execute_from_outside(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
        fee_source: Option<FeeSource>,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        Ok(self.execute_outside(address, outside_execution, signature, fee_source).await?)
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

/// Handles the deployment of a cartridge controller if the estimate fee is requested for a
/// cartridge controller.
///
/// The controller accounts are created with a specific version of the controller.
/// To ensure address determinism, the controller account must be deployed with the same version,
/// which is included in the calldata retrieved from the Cartridge API.
pub async fn get_controller_deploy_tx_if_controller_address(
    paymaster_address: ContractAddress,
    paymaster_private_key: Felt,
    paymaster_nonce: Nonce,
    tx: &ExecutableTxWithHash,
    chain_id: ChainId,
    state: Arc<Box<dyn StateProvider>>,
    cartridge_api_client: &cartridge::Client,
) -> anyhow::Result<Option<ExecutableTxWithHash>> {
    // The whole Cartridge paymaster flow would only be accessible mainly from the Controller
    // wallet. The Controller wallet only supports V3 transactions (considering < V3
    // transactions will soon be deprecated) hence why we're only checking for V3 transactions
    // here.
    //
    // Yes, ideally it's better to handle all versions but it's probably fine for now.
    if let ExecutableTx::Invoke(InvokeTx::V3(v3)) = &tx.transaction {
        let maybe_controller_address = v3.sender_address;

        // Avoid deploying the controller account if it is already deployed.
        if state.class_hash_of_contract(maybe_controller_address)?.is_some() {
            return Ok(None);
        }

        if let tx @ Some(..) = craft_deploy_cartridge_controller_tx(
            cartridge_api_client,
            maybe_controller_address,
            paymaster_address,
            paymaster_private_key,
            chain_id,
            paymaster_nonce,
        )
        .await?
        {
            debug!(address = %maybe_controller_address, "Deploying controller account.");
            return Ok(tx);
        }
    }

    Ok(None)
}

/// Crafts a deploy controller transaction for a cartridge controller.
///
/// Returns None if the provided `controller_address` is not registered in the Cartridge API.
pub async fn craft_deploy_cartridge_controller_tx(
    cartridge_api_client: &cartridge::Client,
    controller_address: ContractAddress,
    paymaster_address: ContractAddress,
    paymaster_private_key: Felt,
    chain_id: ChainId,
    paymaster_nonce: Felt,
) -> anyhow::Result<Option<ExecutableTxWithHash>> {
    if let Some(res) = cartridge_api_client
        .get_account_calldata(controller_address)
        .await
        .map_err(|e| anyhow!("Failed to fetch controller constructor calldata: {e}"))?
    {
        let call = FunctionCall {
            contract_address: DEFAULT_UDC_ADDRESS,
            entry_point_selector: selector!("deployContract"),
            calldata: res.constructor_calldata,
        };

        let mut tx = InvokeTxV3 {
            chain_id,
            tip: 0_u64,
            signature: vec![],
            paymaster_data: vec![],
            account_deployment_data: vec![],
            sender_address: paymaster_address,
            calldata: encode_calls(vec![call]),
            nonce: paymaster_nonce,
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
        };

        let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

        let signer = LocalWallet::from(SigningKey::from_secret_scalar(paymaster_private_key));
        let signature = signer
            .sign_hash(&tx_hash)
            .await
            .map_err(|e| anyhow!("failed to sign hash with paymaster: {e}"))?;
        tx.signature = vec![signature.r, signature.s];

        let tx = ExecutableTxWithHash::new(ExecutableTx::Invoke(InvokeTx::V3(tx)));

        Ok(Some(tx))
    } else {
        Ok(None)
    }
}

fn build_execute_from_outside_call_data(
    address: ContractAddress,
    outside_execution: &OutsideExecution,
    signature: &Vec<Felt>,
) -> katana_rpc_types::outside_execution::Call {
    let entrypoint = match outside_execution {
        OutsideExecution::V2(_) => selector!("execute_from_outside_v2"),
        OutsideExecution::V3(_) => selector!("execute_from_outside_v3"),
    };

    let mut calldata = match outside_execution {
        OutsideExecution::V2(v2) => OutsideExecutionV2::cairo_serialize(v2),
        OutsideExecution::V3(v3) => OutsideExecutionV3::cairo_serialize(v3),
    };

    calldata.extend(Vec::<Felt>::cairo_serialize(signature));

    katana_rpc_types::outside_execution::Call { to: address, selector: entrypoint, calldata }
}

fn build_execute_from_outside_call(
    address: ContractAddress,
    outside_execution: &OutsideExecution,
    signature: &Vec<Felt>,
) -> PaymasterCall {
    let call = build_execute_from_outside_call_data(address, outside_execution, signature);
    PaymasterCall { to: call.to.into(), selector: call.selector, calldata: call.calldata }
}

fn build_execute_from_outside_call_from_vrf_result(
    result: &cartridge::vrf::SignedOutsideExecution,
) -> PaymasterCall {
    let (selector, calldata) = match &result.outside_execution {
        cartridge::vrf::VrfOutsideExecution::V2(v2) => {
            let mut calldata = OutsideExecutionV2::cairo_serialize(v2);
            calldata.extend(Vec::<Felt>::cairo_serialize(&result.signature));
            (selector!("execute_from_outside_v2"), calldata)
        }
        cartridge::vrf::VrfOutsideExecution::V3(v3) => {
            let mut calldata = OutsideExecutionV3::cairo_serialize(v3);
            calldata.extend(Vec::<Felt>::cairo_serialize(&result.signature));
            (selector!("execute_from_outside_v3"), calldata)
        }
    };

    PaymasterCall { to: result.address.into(), selector, calldata }
}
