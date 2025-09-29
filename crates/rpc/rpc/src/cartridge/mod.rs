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

use std::sync::Arc;

use anyhow::anyhow;
use cainome::cairo_serde::CairoSerde;
use cartridge::vrf::{
    VrfContext, CARTRIDGE_VRF_CLASS_HASH, CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY, CARTRIDGE_VRF_SALT,
};
use jsonrpsee::core::{async_trait, RpcResult};
use katana_core::backend::Backend;
use katana_core::service::block_producer::{BlockProducer, BlockProducerMode, PendingExecutor};
use katana_executor::ExecutorFactory;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_pool::{TransactionPool, TxPool};
use katana_primitives::chain::{ChainId, NamedChainId};
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::broadcasted::AddInvokeTransactionResponse;
use katana_rpc_types::outside_execution::{
    OutsideExecution, OutsideExecutionV2, OutsideExecutionV3,
};
use katana_rpc_types::FunctionCall;
use katana_tasks::TokioTaskSpawner;
use starknet::core::types::Call as StarknetCall;
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tracing::{debug, info};
use url::Url;

use paymaster_starknet::transaction::{
    Calls as PaymasterCalls, ExecuteFromOutsideMessage as PaymasterExecuteFromOutsideMessage,
    ExecuteFromOutsideParameters as PaymasterExecuteFromOutsideParameters,
    PaymasterVersion as AvnuPaymasterVersion, TimeBounds as PaymasterTimeBounds,
};
use paymaster_starknet::ChainID as PaymasterChainId;

use super::paymaster::PaymasterService;

#[allow(missing_debug_implementations)]
pub struct CartridgeApi<EF: ExecutorFactory> {
    backend: Arc<Backend<EF>>,
    block_producer: BlockProducer<EF>,
    pool: TxPool,
    vrf_ctx: VrfContext,
    /// The Cartridge API client for paymaster related operations.
    api_client: cartridge::Client,
    paymaster: Option<Arc<PaymasterService>>,
}

impl<EF> Clone for CartridgeApi<EF>
where
    EF: ExecutorFactory,
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            block_producer: self.block_producer.clone(),
            pool: self.pool.clone(),
            api_client: self.api_client.clone(),
            vrf_ctx: self.vrf_ctx.clone(),
            paymaster: self.paymaster.clone(),
        }
    }
}

impl<EF: ExecutorFactory> CartridgeApi<EF> {
    pub fn new(
        backend: Arc<Backend<EF>>,
        block_producer: BlockProducer<EF>,
        pool: TxPool,
        api_url: Url,
        paymaster: Option<Arc<PaymasterService>>,
    ) -> Self {
        // Pulling the paymaster address merely to print the VRF contract address.
        let (pm_address, _) = backend
            .chain_spec
            .genesis()
            .accounts()
            .nth(0)
            .expect("Cartridge paymaster account should exist");

        let api_client = cartridge::Client::new(api_url);
        let vrf_ctx = VrfContext::new(CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY, *pm_address);

        // Info to ensure this is visible to the user without changing the default logging level.
        // The use can still use `rpc::cartridge` in debug to see the random value and the seed.
        info!(target: "rpc::cartridge", paymaster_address = %pm_address, vrf_address = %vrf_ctx.address(), "Cartridge API initialized.");

        Self { backend, block_producer, pool, api_client, vrf_ctx, paymaster }
    }

    fn build_execute_call_via_paymaster(
        &self,
        address: ContractAddress,
        outside_execution: &OutsideExecution,
        signature: &[Felt],
        chain_id: ChainId,
    ) -> Option<FunctionCall> {
        let Some(_paymaster) = self.paymaster.as_ref() else { return None; };
        let avnu_chain_id = map_chain_id(chain_id)?;

        match outside_execution {
            OutsideExecution::V2(v2) => {
                let starknet_calls = v2
                    .calls
                    .iter()
                    .map(|call| StarknetCall {
                        to: (*call.to).into(),
                        selector: call.entry_point_selector,
                        calldata: call.calldata.clone(),
                    })
                    .collect();

                let params = PaymasterExecuteFromOutsideParameters {
                    chain_id: avnu_chain_id,
                    caller: (*v2.caller).into(),
                    nonce: v2.nonce,
                    time_bounds: PaymasterTimeBounds {
                        execute_after: v2.execute_after,
                        execute_before: v2.execute_before,
                    },
                    calls: PaymasterCalls::new(starknet_calls),
                };

                let message =
                    PaymasterExecuteFromOutsideMessage::new(AvnuPaymasterVersion::V2, params);
                let starknet_call = message.to_call(address.into(), &signature.to_vec());

                Some(FunctionCall {
                    contract_address: address,
                    entry_point_selector: starknet_call.selector,
                    calldata: starknet_call.calldata,
                })
            }
            OutsideExecution::V3(_) => None,
        }
    }

    fn nonce(&self, contract_address: ContractAddress) -> Result<Option<Nonce>, StarknetApiError> {
        Ok(self.pool.validator().pool_nonce(contract_address)?)
    }

    fn pending_executor(&self) -> Option<PendingExecutor> {
        match &*self.block_producer.producer.read() {
            BlockProducerMode::Instant(_) => None,
            BlockProducerMode::Interval(producer) => Some(producer.executor()),
        }
    }

    pub async fn execute_outside(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
    ) -> Result<AddInvokeTransactionResponse, StarknetApiError> {
        debug!(%address, ?outside_execution, "Adding execute outside transaction.");
        self.on_io_blocking_task(move |this| {
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

            // ====================== CONTROLLER DEPLOYMENT ======================
            // Check if the controller is already deployed. If not, deploy it.

            let is_controller_deployed = {
	            match this.pending_executor().as_ref() {
	                Some(executor) => executor.read().state().class_hash_of_contract(address)?.is_some(),
	                None => {
						let provider = this.backend.blockchain.provider();
						provider.latest()?.class_hash_of_contract(address)?.is_some()},
	            }
            };

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
                    this.pool.add_transaction(tx)?;
                }
            }

            // ===================================================================

            // If we submitted a deploy Controller transaction, then the execute from outside
            // transaction nonce should be incremented.
            if !is_controller_deployed {
                nonce += Nonce::ONE;
            }

            let chain_id = this.backend.chain_spec.id();

            let execute_from_outside_call = if let Some(call) = this.build_execute_call_via_paymaster(
                address,
                &outside_execution,
                &signature,
                chain_id,
            ) {
                call
            } else {
                let mut inner_calldata = match &outside_execution {
                    OutsideExecution::V2(v2) => OutsideExecutionV2::cairo_serialize(v2),
                    OutsideExecution::V3(v3) => OutsideExecutionV3::cairo_serialize(v3),
                };

                inner_calldata.extend(Vec::<Felt>::cairo_serialize(&signature));

                FunctionCall { contract_address: address, entry_point_selector: entrypoint, calldata: inner_calldata }
            };

            // ======= VRF checks =======

            let state = this.backend.blockchain.provider().latest().map(Arc::new)?;

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
                this.pool.add_transaction(tx)?;

                // Ensure the nonce is increment for execution from outside.
                nonce += Nonce::ONE;
            }

            let vrf_calls = futures::executor::block_on(handle_vrf_calls(&outside_execution, chain_id, &this.vrf_ctx))?;

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
            let transaction_hash = this.pool.add_transaction(tx)?;

            Ok(AddInvokeTransactionResponse {transaction_hash})
        })
        .await
    }

    async fn on_io_blocking_task<F, T>(&self, func: F) -> T
    where
        F: FnOnce(Self) -> T + Send + 'static,
        T: Send + 'static,
    {
        let this = self.clone();
        TokioTaskSpawner::new().unwrap().spawn_blocking(move || func(this)).await.unwrap()
    }
}

fn map_chain_id(chain_id: ChainId) -> Option<PaymasterChainId> {
    match chain_id {
        ChainId::Named(NamedChainId::Mainnet) => Some(PaymasterChainId::Mainnet),
        ChainId::Named(NamedChainId::Sepolia) => Some(PaymasterChainId::Sepolia),
        _ => None,
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
        let contract_address = salt_or_nonce;
        let nonce = vrf_ctx.consume_nonce(contract_address.into());
        starknet_crypto::poseidon_hash_many(vec![&nonce, &caller, &chain_id.id()])
    } else if salt_or_nonce_selector == Felt::ONE {
        let salt = salt_or_nonce;
        starknet_crypto::poseidon_hash_many(vec![&salt, &caller, &chain_id.id()])
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
        CARTRIDGE_VRF_SALT,
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
