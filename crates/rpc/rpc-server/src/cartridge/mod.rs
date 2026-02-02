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
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use cainome::cairo_serde::CairoSerde;
use cainome::cairo_serde_derive::CairoSerde as CairoSerdeDerive;
use cainome_cairo_serde::ContractAddress as CairoContractAddress;
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
use katana_primitives::{felt, ContractAddress, Felt};
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
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use starknet_crypto::{pedersen_hash, poseidon_hash_many, PoseidonHasher};
use starknet_paymaster::core::types::Call as PaymasterCall;
use tracing::{debug, info};
use url::Url;

#[derive(Debug, Clone)]
pub struct CartridgeConfig {
    pub cartridge_api_url: Url,
    pub paymaster_url: Url,
    pub paymaster_api_key: Option<String>,
    pub paymaster_address: ContractAddress,
    pub paymaster_private_key: Felt,
    pub vrf: Option<CartridgeVrfConfig>,
}

#[derive(Debug, Clone)]
pub struct CartridgeVrfConfig {
    pub url: Url,
    pub account_address: ContractAddress,
    pub account_private_key: Felt,
}

#[derive(Clone)]
struct VrfService {
    client: ReqwestClient,
    url: Url,
    account_address: ContractAddress,
    account_private_key: Felt,
}

impl VrfService {
    fn new(config: CartridgeVrfConfig) -> Self {
        Self {
            client: ReqwestClient::new(),
            url: config.url,
            account_address: config.account_address,
            account_private_key: config.account_private_key,
        }
    }

    async fn prove(&self, seed: Felt) -> Result<VrfProof, StarknetApiError> {
        let endpoint = format!("{}/stark_vrf", self.url.as_str().trim_end_matches('/'));
        let payload = VrfServerRequest { seed: vec![seed.to_hex_string()] };

        let response =
            self.client.post(endpoint).json(&payload).send().await.map_err(|err| {
                StarknetApiError::unexpected(format!("vrf request failed: {err}"))
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            StarknetApiError::unexpected(format!("vrf response read failed: {err}"))
        })?;

        if !status.is_success() {
            return Err(StarknetApiError::unexpected(format!(
                "vrf service error ({status}): {body}"
            )));
        }

        let result: VrfServerResponse = serde_json::from_str(&body).map_err(|err| {
            StarknetApiError::unexpected(format!("vrf response parse failed: {err}"))
        })?;

        Ok(VrfProof {
            gamma_x: parse_felt(&result.result.gamma_x)?,
            gamma_y: parse_felt(&result.result.gamma_y)?,
            c: parse_felt(&result.result.c)?,
            s: parse_felt(&result.result.s)?,
            sqrt_ratio: parse_felt(&result.result.sqrt_ratio)?,
        })
    }
}

#[derive(Debug, Serialize)]
struct VrfServerRequest {
    seed: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct VrfServerResponse {
    result: VrfServerResult,
}

#[derive(Debug, Deserialize)]
struct VrfServerResult {
    gamma_x: String,
    gamma_y: String,
    c: String,
    s: String,
    sqrt_ratio: String,
}

#[derive(Debug, Clone)]
struct VrfProof {
    gamma_x: Felt,
    gamma_y: Felt,
    c: Felt,
    s: Felt,
    sqrt_ratio: Felt,
}

#[derive(Clone, CairoSerdeDerive, Serialize, Deserialize, Debug)]
enum VrfSource {
    Nonce(CairoContractAddress),
    Salt(Felt),
}

#[derive(Clone, CairoSerdeDerive, Serialize, Deserialize, Debug)]
struct VrfRequestRandom {
    caller: CairoContractAddress,
    source: VrfSource,
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
    paymaster_address: ContractAddress,
    /// The paymaster account private key.
    paymaster_private_key: Felt,
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
            paymaster_address: self.paymaster_address,
            paymaster_private_key: self.paymaster_private_key,
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
        let api_client = cartridge::Client::new(config.cartridge_api_url);
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
            paymaster_address: config.paymaster_address,
            paymaster_private_key: config.paymaster_private_key,
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
            let pm_address = this.paymaster_address;
            let pm_private_key = this.paymaster_private_key;

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
                    if request_random_call.to != vrf_service.account_address {
                        return Err(StarknetApiError::unexpected(
                            "request_random call must target the vrf account",
                        ));
                    }

                    let request_random =
                        VrfRequestRandom::cairo_deserialize(&request_random_call.calldata, 0)
                            .map_err(|err| {
                                StarknetApiError::unexpected(format!(
                                    "vrf request_random decode failed: {err}"
                                ))
                            })?;

                    let chain_id = this.backend.chain_spec.id();
                    let seed = compute_vrf_seed(
                        state.as_ref(),
                        vrf_service.account_address,
                        &request_random,
                        chain_id.id(),
                    )?;
                    let proof = vrf_service.prove(seed).await?;

                    let submit_random_call =
                        build_submit_random_call(vrf_service.account_address, seed, &proof);
                    let execute_from_outside_call_data =
                        build_execute_from_outside_call_data(address, &outside_execution, &signature);

                    let (wrapped_execution, wrapped_signature) = build_vrf_outside_execution(
                        vrf_service.account_address,
                        vrf_service.account_private_key,
                        chain_id,
                        vec![submit_random_call, execute_from_outside_call_data],
                    )
                    .await?;

                    user_address = vrf_service.account_address.into();
                    execute_from_outside_call = build_execute_from_outside_call(
                        vrf_service.account_address,
                        &OutsideExecution::V2(wrapped_execution),
                        &wrapped_signature,
                    );
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

const STARKNET_DOMAIN_TYPE_HASH: Felt =
    Felt::from_hex_unchecked("0x1ff2f602e42168014d405a94f75e8a93d640751d71d16311266e140d8b0a210");
const CALL_TYPE_HASH: Felt =
    Felt::from_hex_unchecked("0x3635c7f2a7ba93844c0d064e18e487f35ab90f7c39d00f186a781fc3f0c2ca9");
const OUTSIDE_EXECUTION_TYPE_HASH: Felt =
    Felt::from_hex_unchecked("0x312b56c05a7965066ddbda31c016d8d05afc305071c0ca3cdc2192c3c2f1f0f");
const ANY_CALLER: Felt = felt!("0x414e595f43414c4c4552");

fn request_random_call(
    outside_execution: &OutsideExecution,
) -> Option<(katana_rpc_types::outside_execution::Call, usize)> {
    let calls = match outside_execution {
        OutsideExecution::V2(v2) => &v2.calls,
        OutsideExecution::V3(v3) => &v3.calls,
    };

    calls
        .iter()
        .position(|call| call.selector == selector!("request_random"))
        .map(|position| (calls[position].clone(), position))
}

fn outside_execution_calls_len(outside_execution: &OutsideExecution) -> usize {
    match outside_execution {
        OutsideExecution::V2(v2) => v2.calls.len(),
        OutsideExecution::V3(v3) => v3.calls.len(),
    }
}

fn compute_vrf_seed(
    state: &dyn StateProvider,
    vrf_account_address: ContractAddress,
    request_random: &VrfRequestRandom,
    chain_id: Felt,
) -> Result<Felt, StarknetApiError> {
    let caller = request_random.caller.0;

    match &request_random.source {
        VrfSource::Nonce(contract_address) => {
            let storage_key = pedersen_hash(&selector!("VrfProvider_nonces"), &contract_address.0);
            let nonce = state.storage(vrf_account_address, storage_key)?.unwrap_or_default();
            Ok(poseidon_hash_many(&[nonce, contract_address.0, caller, chain_id]))
        }
        VrfSource::Salt(salt) => Ok(poseidon_hash_many(&[*salt, caller, chain_id])),
    }
}

fn build_submit_random_call(
    vrf_account_address: ContractAddress,
    seed: Felt,
    proof: &VrfProof,
) -> katana_rpc_types::outside_execution::Call {
    katana_rpc_types::outside_execution::Call {
        to: vrf_account_address,
        selector: selector!("submit_random"),
        calldata: vec![seed, proof.gamma_x, proof.gamma_y, proof.c, proof.s, proof.sqrt_ratio],
    }
}

async fn build_vrf_outside_execution(
    account_address: ContractAddress,
    account_private_key: Felt,
    chain_id: ChainId,
    calls: Vec<katana_rpc_types::outside_execution::Call>,
) -> Result<(OutsideExecutionV2, Vec<Felt>), StarknetApiError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| StarknetApiError::unexpected(format!("clock error: {err}")))?
        .as_secs();
    let outside_execution = OutsideExecutionV2 {
        caller: ContractAddress::from(ANY_CALLER),
        execute_after: 0,
        execute_before: now + 600,
        calls,
        nonce: SigningKey::from_random().secret_scalar(),
    };

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(account_private_key));
    let signature =
        sign_outside_execution_v2(&outside_execution, chain_id.id(), account_address, signer)
            .await?;

    Ok((outside_execution, signature))
}

async fn sign_outside_execution_v2(
    outside_execution: &OutsideExecutionV2,
    chain_id: Felt,
    signer_address: ContractAddress,
    signer: LocalWallet,
) -> Result<Vec<Felt>, StarknetApiError> {
    let mut final_hasher = PoseidonHasher::new();
    final_hasher.update(Felt::from_bytes_be_slice(b"StarkNet Message"));
    final_hasher.update(starknet_domain_hash(chain_id));
    final_hasher.update(signer_address.into());
    final_hasher.update(outside_execution_hash(outside_execution));

    let hash = final_hasher.finalize();
    let signature = signer
        .sign_hash(&hash)
        .await
        .map_err(|e| StarknetApiError::unexpected(format!("failed to sign vrf execution: {e}")))?;

    Ok(vec![signature.r, signature.s])
}

fn starknet_domain_hash(chain_id: Felt) -> Felt {
    let domain = [
        STARKNET_DOMAIN_TYPE_HASH,
        Felt::from_bytes_be_slice(b"Account.execute_from_outside"),
        Felt::TWO,
        chain_id,
        Felt::ONE,
    ];
    poseidon_hash_many(&domain)
}

fn outside_execution_hash(outside_execution: &OutsideExecutionV2) -> Felt {
    let hashed_calls: Vec<Felt> = outside_execution.calls.iter().map(call_hash).collect();

    let mut hasher = PoseidonHasher::new();
    hasher.update(OUTSIDE_EXECUTION_TYPE_HASH);
    hasher.update(outside_execution.caller.into());
    hasher.update(outside_execution.nonce);
    hasher.update(Felt::from(outside_execution.execute_after));
    hasher.update(Felt::from(outside_execution.execute_before));
    hasher.update(poseidon_hash_many(&hashed_calls));
    hasher.finalize()
}

fn call_hash(call: &katana_rpc_types::outside_execution::Call) -> Felt {
    let mut hasher = PoseidonHasher::new();
    hasher.update(CALL_TYPE_HASH);
    hasher.update(call.to.into());
    hasher.update(call.selector);
    hasher.update(poseidon_hash_many(&call.calldata));
    hasher.finalize()
}

fn parse_felt(value: &str) -> Result<Felt, StarknetApiError> {
    if value.starts_with("0x") || value.starts_with("0X") {
        Felt::from_hex(value).map_err(|err| {
            StarknetApiError::unexpected(format!("invalid felt hex '{value}': {err}"))
        })
    } else {
        Felt::from_dec_str(value).map_err(|err| {
            StarknetApiError::unexpected(format!("invalid felt decimal '{value}': {err}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::str::FromStr;

    use katana_primitives::contract::{StorageKey, StorageValue};
    use katana_provider::api::contract::ContractClassProvider;
    use katana_provider::api::state::{StateProofProvider, StateProvider, StateRootProvider};
    use katana_provider::ProviderResult;
    use stark_vrf::{generate_public_key, BaseField, ScalarField, StarkVRF};
    use starknet::macros::selector;
    use starknet_crypto::{pedersen_hash, poseidon_hash_many};

    use super::*;

    fn felt_from_display<T: std::fmt::Display>(value: T) -> Felt {
        Felt::from_dec_str(&value.to_string()).expect("valid felt")
    }

    #[test]
    fn request_random_call_finds_position() {
        let vrf_address = ContractAddress::from(felt!("0x123"));
        let other_call = katana_rpc_types::outside_execution::Call {
            to: vrf_address,
            selector: selector!("transfer"),
            calldata: vec![Felt::ONE],
        };
        let vrf_call = katana_rpc_types::outside_execution::Call {
            to: vrf_address,
            selector: selector!("request_random"),
            calldata: vec![Felt::TWO],
        };

        let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
            caller: ContractAddress::from(ANY_CALLER),
            execute_after: 0,
            execute_before: 100,
            calls: vec![other_call.clone(), vrf_call.clone()],
            nonce: Felt::THREE,
        });

        let (call, position) =
            request_random_call(&outside_execution).expect("request_random found");
        assert_eq!(position, 1);
        assert_eq!(call.selector, vrf_call.selector);
        assert_eq!(call.calldata, vrf_call.calldata);
    }

    #[test]
    fn submit_random_call_matches_proof() {
        let secret_key = Felt::from(0x123_u128);
        let secret_key_scalar =
            ScalarField::from_str(&secret_key.to_biguint().to_str_radix(10)).unwrap();
        let public_key = generate_public_key(secret_key_scalar);
        let vrf_account_address = ContractAddress::from(Felt::from(0x456_u128));

        let seed = Felt::from(0xabc_u128);
        let seed_vec = vec![BaseField::from_str(&seed.to_biguint().to_str_radix(10)).unwrap()];
        let ecvrf = StarkVRF::new(public_key).unwrap();
        let proof = ecvrf.prove(&secret_key_scalar, seed_vec.as_slice()).unwrap();
        let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed_vec.as_slice());

        let vrf_proof = VrfProof {
            gamma_x: felt_from_display(proof.0.x),
            gamma_y: felt_from_display(proof.0.y),
            c: felt_from_display(proof.1),
            s: felt_from_display(proof.2),
            sqrt_ratio: felt_from_display(sqrt_ratio_hint),
        };

        let call = build_submit_random_call(vrf_account_address, seed, &vrf_proof);

        let expected = vec![
            seed,
            felt_from_display(proof.0.x),
            felt_from_display(proof.0.y),
            felt_from_display(proof.1),
            felt_from_display(proof.2),
            felt_from_display(sqrt_ratio_hint),
        ];

        assert_eq!(call.selector, selector!("submit_random"));
        assert_eq!(call.to, vrf_account_address);
        assert_eq!(call.calldata, expected);
    }

    #[derive(Default)]
    struct StubState {
        storage: HashMap<(ContractAddress, StorageKey), StorageValue>,
    }

    impl ContractClassProvider for StubState {
        fn class(
            &self,
            _hash: katana_primitives::class::ClassHash,
        ) -> ProviderResult<Option<katana_primitives::class::ContractClass>> {
            Ok(None)
        }

        fn compiled_class_hash_of_class_hash(
            &self,
            _hash: katana_primitives::class::ClassHash,
        ) -> ProviderResult<Option<katana_primitives::class::CompiledClassHash>> {
            Ok(None)
        }
    }

    impl StateRootProvider for StubState {}
    impl StateProofProvider for StubState {}

    impl StateProvider for StubState {
        fn nonce(&self, _address: ContractAddress) -> ProviderResult<Option<Felt>> {
            Ok(None)
        }

        fn storage(
            &self,
            address: ContractAddress,
            storage_key: StorageKey,
        ) -> ProviderResult<Option<StorageValue>> {
            Ok(self.storage.get(&(address, storage_key)).copied())
        }

        fn class_hash_of_contract(
            &self,
            _address: ContractAddress,
        ) -> ProviderResult<Option<katana_primitives::class::ClassHash>> {
            Ok(None)
        }
    }

    #[test]
    fn compute_vrf_seed_uses_nonce_storage() {
        let vrf_account_address = ContractAddress::from(Felt::from(0x100_u128));
        let caller = CairoContractAddress(Felt::from(0x200_u128));
        let source = CairoContractAddress(Felt::from(0x300_u128));
        let request = VrfRequestRandom { caller, source: VrfSource::Nonce(source) };

        let storage_key = pedersen_hash(&selector!("VrfProvider_nonces"), &source.0);
        let nonce = Felt::from(0x1234_u128);

        let mut state = StubState::default();
        state.storage.insert((vrf_account_address, storage_key), nonce);

        let chain_id = Felt::from(0x534e5f4d41494e_u128);
        let seed = compute_vrf_seed(&state, vrf_account_address, &request, chain_id).expect("seed");

        let expected = poseidon_hash_many(&[nonce, source.0, caller.0, chain_id]);
        assert_eq!(seed, expected);
    }

    #[test]
    fn compute_vrf_seed_uses_salt() {
        let vrf_account_address = ContractAddress::from(Felt::from(0x100_u128));
        let caller = CairoContractAddress(Felt::from(0x200_u128));
        let salt = Felt::from(0x999_u128);
        let request = VrfRequestRandom { caller, source: VrfSource::Salt(salt) };

        let state = StubState::default();
        let chain_id = Felt::from(0x534e5f4d41494e_u128);
        let seed = compute_vrf_seed(&state, vrf_account_address, &request, chain_id).expect("seed");

        let expected = poseidon_hash_many(&[salt, caller.0, chain_id]);
        assert_eq!(seed, expected);
    }

    #[test]
    fn parse_felt_accepts_hex_and_decimal() {
        assert_eq!(parse_felt("0x10").expect("hex"), Felt::from(0x10_u128));
        assert_eq!(parse_felt("42").expect("decimal"), Felt::from(42_u128));
    }

    #[test]
    fn parse_felt_rejects_invalid_strings() {
        let err = parse_felt("not-a-felt").unwrap_err();
        match err {
            StarknetApiError::UnexpectedError(data) => {
                assert!(data.reason.contains("invalid felt"), "unexpected error: {data:?}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
