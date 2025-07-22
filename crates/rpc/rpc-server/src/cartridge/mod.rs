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
use katana_primitives::chain::ChainId;
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::hash::{Pedersen, Poseidon, StarkHash};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
<<<<<<<< HEAD:crates/rpc/rpc-server/src/cartridge/mod.rs
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::broadcasted::AddInvokeTransactionResponse;
use katana_rpc_types::outside_execution::{
    OutsideExecution, OutsideExecutionV2, OutsideExecutionV3,
};
use katana_rpc_types::FunctionCall;
use katana_tasks::{Result as TaskResult, TaskSpawner};
========
use katana_provider::traits::state::{StateFactoryProvider, StateProvider};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::transaction::InvokeTxResult;
use katana_tasks::TokioTaskSpawner;
use starknet::core::types::Call;
>>>>>>>> 2d078ce3 (update):crates/cartridge/src/rpc/mod.rs
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tracing::{debug, info};
use url::Url;

mod api;
mod types;

pub use api::*;
pub use types::*;

use crate::vrf::{
    VrfContext, CARTRIDGE_VRF_CLASS_HASH, CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY, CARTRIDGE_VRF_SALT,
};
use crate::Client as CartridgeClient;

#[allow(missing_debug_implementations)]
pub struct CartridgeApi<EF: ExecutorFactory, PF: ProviderFactory> {
    task_spawner: TaskSpawner,
    backend: Arc<Backend<EF, PF>>,
    block_producer: BlockProducer<EF, PF>,
    pool: TxPool,
    vrf_ctx: VrfContext,
    /// The Cartridge API client for paymaster related operations.
    api_client: CartridgeClient,
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
            vrf_ctx: self.vrf_ctx.clone(),
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
        api_url: Url,
    ) -> Self {
        // Pulling the paymaster address merely to print the VRF contract address.
        let (pm_address, _) = backend
            .chain_spec
            .genesis()
            .accounts()
            .nth(0)
            .expect("Cartridge paymaster account should exist");

        let api_client = CartridgeClient::new(api_url);
        let vrf_ctx = VrfContext::new(CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY, *pm_address);
        // Info to ensure this is visible to the user without changing the default logging level.
        // The use can still use `rpc::cartridge` in debug to see the random value and the seed.
        info!(target: "rpc::cartridge", paymaster_address = %pm_address, vrf_address = %vrf_ctx.address(), "Cartridge API initialized.");

        Self { task_spawner, backend, block_producer, pool, api_client, vrf_ctx }
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
                return Err(StarknetApiError::unexpected("Paymaster is not a dev account"))
            };

            // Contract function selector for
            let entrypoint = match outside_execution {
                OutsideExecution::V2(_) => selector!("execute_from_outside_v2"),
                OutsideExecution::V3(_) => selector!("execute_from_outside_v3"),
            };

            // Get the current nonce of the paymaster account.
            let mut nonce = this.nonce(*pm_address)?.unwrap_or_default();


<<<<<<<< HEAD:crates/rpc/rpc-server/src/cartridge/mod.rs
            let state = this.state().map(Arc::new)?;
            let is_controller_deployed = state.class_hash_of_contract(address)?.is_some();

            if !is_controller_deployed {
	           	debug!(target: "rpc::cartridge", controller = %address, "Controller not yet deployed");
                if let Some(tx) =
                    futures::executor::block_on(craft_deploy_cartridge_controller_tx(
                        &this.api_client,
                        address,
                        *pm_address,
                        pm_private_key,
                        this.backend.chain_spec.id(),
                        nonce,
                    ))?
                {
                	debug!(target: "rpc::cartridge", controller = %address, tx = format!("{:#x}", tx.hash),  "Inserting Controller deployment transaction");
                    this.pool.add_transaction(tx).await?;
                }
            }

            // ===================================================================

            // If we submitted a deploy Controller transaction, then the execute from outside
            // transaction nonce should be incremented.
            if !is_controller_deployed {
                nonce += Nonce::ONE;
            }

            let mut inner_calldata = match &outside_execution {
                OutsideExecution::V2(v2) => {
                    OutsideExecutionV2::cairo_serialize(v2)
                }
                OutsideExecution::V3(v3) => {
                    OutsideExecutionV3::cairo_serialize(v3)
                }
            };
========
>>>>>>>> 2d078ce3 (update):crates/cartridge/src/rpc/mod.rs

            let mut inner_calldata = OutsideExecution::cairo_serialize(&outside_execution);
            inner_calldata.extend(Vec::<Felt>::cairo_serialize(&signature));

            let execute_from_outside_call = FunctionCall { contract_address: address, entry_point_selector: entrypoint, calldata: inner_calldata };

            let chain_id = this.backend.chain_spec.id();

            // ======= VRF checks =======

            let (public_key_x, public_key_y) = this.vrf_ctx.get_public_key_xy_felts();
            let vrf_address = this.vrf_ctx.address();

            let class_hash = state.class_hash_of_contract(vrf_address)?;
            if class_hash.is_none() {
                let tx = futures::executor::block_on(craft_deploy_cartridge_vrf_tx(
                    katana_primitives::ContractAddress(**pm_address),
                    pm_private_key,
                    chain_id,
                    nonce,
                    public_key_x,
                    public_key_y,
                ))?;

                debug!(target: "rpc::cartridge", controller = %address, tx = format!("{:#x}", tx.hash),  "Inserting Cartridge VRF deployment transaction.");
                this.pool.add_transaction(tx).await?;

                // Ensure the nonce is increment for execution from outside.
                nonce += Nonce::ONE;
            }

            let vrf_calls = futures::executor::block_on(handle_vrf_calls(&outside_execution, chain_id, &this.vrf_ctx, state))?;

            let calls = if vrf_calls.is_empty() {
                vec![execute_from_outside_call]
            } else {
                assert!(vrf_calls.len() == 2);
                // First call to submit randomness, execution from outside must consume it, and final call to assert consumption.
                vec![vrf_calls[0].clone(), execute_from_outside_call, vrf_calls[1].clone()]
            };

            let mut tx = InvokeTxV3 {
                nonce,
                chain_id,
                calldata: encode_calls(calls),
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

            Ok(AddInvokeTransactionResponse {transaction_hash})
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
    cartridge_api_client: &CartridgeClient,
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
    cartridge_api_client: &CartridgeClient,
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

/// Inspects the [`OutsideExecution`] to search for `request_random` call sent to the VRF contract
/// as the first call.
///
/// If it's a VRF call, the calls to submit randomness and assert consumption are returned.
/// Otherwise, an empty vector is returned.
///
/// In the current implementation, Katana doesn't store the cached nonces into the database, so any
/// restart of Katana would result in a reset of this nonce (hence predictable VRF).
async fn handle_vrf_calls(
    outside_execution: &OutsideExecution,
    chain_id: ChainId,
    vrf_ctx: &VrfContext,
    state: Arc<Box<dyn StateProvider>>,
) -> anyhow::Result<Vec<FunctionCall>> {
    let calls = match outside_execution {
        OutsideExecution::V2(v2) => &v2.calls,
        OutsideExecution::V3(v3) => &v3.calls,
    };

    if calls.is_empty() {
        anyhow::bail!("No calls in outside execution.");
    }

    // Refer to the module documentation for why this is expected and
    // cartridge documentation for more details: <https://docs.cartridge.gg/vrf/overview#executing-vrf-transactions>.
    let first_call = calls.first().unwrap();

    if first_call.selector != selector!("request_random") && first_call.to != vrf_ctx.address() {
        return Ok(Vec::new());
    }

    if first_call.calldata.len() != 3 {
        anyhow::bail!("Invalid calldata for request_random: {:?}", first_call.calldata);
    }

    let caller = first_call.calldata[0];
    let salt_or_nonce_selector = first_call.calldata[1];
    // Salt or nonce being the salt for the `Salt` variant, and the contract address for the `Nonce`
    // variant.
    let salt_or_nonce = first_call.calldata[2];

    let seed = if salt_or_nonce_selector == Felt::ZERO {
        // compute storage key of the VRF contract storage member VrfProvider_nonces:
        // Map<ContractAddress, felt252>
        let address = salt_or_nonce;
        let key = Pedersen::hash(&selector!("VrfProvider_nonces"), &address);

        let nonce = state.storage(vrf_ctx.address(), key).unwrap_or_default().unwrap_or_default();
        Poseidon::hash_array(&[nonce, caller, chain_id.id()])
    } else if salt_or_nonce_selector == Felt::ONE {
        let salt = salt_or_nonce;
        Poseidon::hash_array(&[salt, caller, chain_id.id()])
    } else {
        anyhow::bail!(
            "Invalid salt or nonce for VRF request, expecting 0 or 1, got {}",
            salt_or_nonce_selector
        );
    };

    let proof = vrf_ctx.stark_vrf(seed)?;

    let submit_random_call = FunctionCall {
        contract_address: vrf_ctx.address(),
        entry_point_selector: selector!("submit_random"),
        calldata: vec![
            seed,
            Felt::from_hex_unchecked(&proof.gamma_x),
            Felt::from_hex_unchecked(&proof.gamma_y),
            Felt::from_hex_unchecked(&proof.c),
            Felt::from_hex_unchecked(&proof.s),
            Felt::from_hex_unchecked(&proof.sqrt_ratio),
        ],
    };

    let assert_consumed_call = FunctionCall {
        entry_point_selector: selector!("assert_consumed"),
        contract_address: vrf_ctx.address(),
        calldata: vec![seed],
    };

    Ok(vec![submit_random_call, assert_consumed_call])
}

/// Crafts a deploy of the VRF provider contract transaction.
pub async fn craft_deploy_cartridge_vrf_tx(
    paymaster_address: ContractAddress,
    paymaster_private_key: Felt,
    chain_id: ChainId,
    paymaster_nonce: Felt,
    public_key_x: Felt,
    public_key_y: Felt,
) -> anyhow::Result<ExecutableTxWithHash> {
    let calldata = vec![
        CARTRIDGE_VRF_CLASS_HASH,
        CARTRIDGE_VRF_SALT.into(),
        // from zero
        Felt::ZERO,
        // Calldata len
        Felt::THREE,
        // owner
        paymaster_address.into(),
        // public key
        public_key_x,
        public_key_y,
    ];

    let call = FunctionCall {
        contract_address: DEFAULT_UDC_ADDRESS,
        entry_point_selector: selector!("deployContract"),
        calldata,
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

    Ok(tx)
}
