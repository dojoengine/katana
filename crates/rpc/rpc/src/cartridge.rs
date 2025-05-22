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
//!
//!
//! For the VRF, the inegration works as follows (if an execution from outside is targetting the
//! VRF provider contract):
//!
//! 1. The VRF provider contract is deployed (if not already deployed).
//! 2. The Stark VRF proof is generated using the `Source` provided in the call. Since one of the
//!    `Source` is a nonce for a given address, Katana keeps an in-memory cache of the nonces for
//!    each address. WARNING: Restarting Katana will reset the cache, hence reset the VRF sequence.
//! 3. The original execution from outside call is then sandwitched between two VRF calls, one for
//!    submitting the randomness, and one to assert the correct consumption of the randomness.
//!
//! In the current implementation, the VRF contract is deployed with a default private key, or read
//! from environment variable `KATANA_VRF_PRIVATE_KEY`. It is important to note that changing the
//! private key will result in a different VRF provider contract address.
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use account_sdk::account::outside_execution::OutsideExecution;
use anyhow::anyhow;
use cainome::cairo_serde::CairoSerde;
use jsonrpsee::core::{async_trait, RpcResult};
use katana_core::backend::Backend;
use katana_core::service::block_producer::{BlockProducer, BlockProducerMode, PendingExecutor};
use katana_executor::ExecutorFactory;
use katana_pool::{TransactionPool, TxPool};
use katana_primitives::chain::ChainId;
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::ResourceBoundsMapping;
use katana_primitives::genesis::allocation::GenesisAccountAlloc;
use katana_primitives::genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
use katana_provider::traits::state::{StateFactoryProvider, StateProvider};
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::transaction::InvokeTxResult;
use katana_tasks::TokioTaskSpawner;
use num_bigint::BigInt;
use serde::Deserialize;
use stark_vrf::{generate_public_key, BaseField, StarkVRF};
use starknet::core::types::Call;
use starknet::core::utils::get_contract_address;
use starknet::macros::{felt, selector};
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tracing::{debug, info};
use url::Url;

// Class hash of the VRF provider contract (fee estimation code commented, since currently Katana
// returns 0 for the fees): <https://github.com/cartridge-gg/vrf/blob/38d71385f939a19829113c122f1ab12dbbe0f877/src/vrf_provider/vrf_provider_component.cairo#L124>
// The contract is compiled in
// `crates/controller/artifacts/cartridge_vrf_VrfProvider.contract_class.json`
pub const CARTIDGE_VRF_CLASS_HASH: Felt =
    felt!("0x07007ea60938ff539f1c0772a9e0f39b4314cfea276d2c22c29a8b64f2a87a58");
// Short string for `cartridge_vrf`.
pub const CARTIDGE_VRF_SALT: Felt = felt!("0x6361727472696467655f767266");
pub const CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY: Felt = felt!("0x01");

#[derive(Debug, Default, Clone)]
pub struct StarkVrfProof {
    pub gamma_x: String,
    pub gamma_y: String,
    pub c: String,
    pub s: String,
    pub sqrt_ratio: String,
    pub rnd: String,
}

#[derive(Debug, Default, Clone)]
pub struct VrfContext {
    pub cache: Arc<RwLock<HashMap<Felt, Felt>>>,
    pub private_key: Felt,
}

impl VrfContext {
    /// Get the public key x and y coordinates from the private key.
    pub fn get_public_key_xy(&self) -> (Felt, Felt) {
        let pk_str = self.private_key.to_string();
        let public_key = generate_public_key(pk_str.parse().unwrap());

        let x = Felt::from_str(&public_key.x.to_string()).unwrap();
        let y = Felt::from_str(&public_key.y.to_string()).unwrap();

        (x, y)
    }

    /// Get the VRF contract address from the paymaster address and the public key coordinates.
    pub fn get_vrf_address(&self, pm_address: ContractAddress) -> ContractAddress {
        let (public_key_x, public_key_y) = self.get_public_key_xy();

        get_contract_address(
            CARTIDGE_VRF_SALT,
            CARTIDGE_VRF_CLASS_HASH,
            &[*pm_address, public_key_x, public_key_y],
            Felt::ZERO,
        )
        .into()
    }
}

#[allow(missing_debug_implementations)]
pub struct CartridgeApi<EF: ExecutorFactory> {
    backend: Arc<Backend<EF>>,
    block_producer: BlockProducer<EF>,
    pool: TxPool,
    /// The root URL for the Cartridge API for paymaster related operations.
    api_url: Url,
    vrf_ctx: VrfContext,
}

impl<EF> Clone for CartridgeApi<EF>
where
    EF: ExecutorFactory,
{
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            block_producer: self.block_producer.clone(),
            pool: self.pool.clone(),
            api_url: self.api_url.clone(),
            vrf_ctx: self.vrf_ctx.clone(),
        }
    }
}

impl<EF: ExecutorFactory> CartridgeApi<EF> {
    pub fn new(
        backend: Arc<Backend<EF>>,
        block_producer: BlockProducer<EF>,
        pool: TxPool,
        api_url: Url,
        vrf_cache: Arc<RwLock<HashMap<Felt, Felt>>>,
    ) -> Self {
        let vrf_ctx =
            VrfContext { cache: vrf_cache, private_key: CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY };

        // Pulling the paymaster address merely to print the VRF contract address.
        let (pm_address, _) = backend
            .chain_spec
            .genesis()
            .accounts()
            .nth(0)
            .expect("Cartridge paymaster account should exist");

        let vrf_contract_address = vrf_ctx.get_vrf_address(*pm_address);

        // Info to ensure this is visible to the user without changing the default logging level.
        // The use can still use `rpc::cartridge` in debug to see the random value and the seed.
        info!(target: "rpc::cartridge", paymaster_address = %pm_address, vrf_address = %vrf_contract_address, "Cartridge API initialized.");

        Self { backend, block_producer, pool, api_url, vrf_ctx }
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
    ) -> Result<InvokeTxResult, StarknetApiError> {
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
                let reason = "Paymaster is not a dev account".to_string();
                return Err(StarknetApiError::UnexpectedError { reason });
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
                        &this.api_url,
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

            let mut inner_calldata =
                <OutsideExecution as CairoSerde>::cairo_serialize(&outside_execution);
            inner_calldata.extend(<Vec<Felt> as CairoSerde>::cairo_serialize(&signature));

            let execute_from_outside_call = Call { to: address.into(), selector: entrypoint, calldata: inner_calldata };

            let chain_id = this.backend.chain_spec.id();

            // ======= VRF checks =======

            let state = this.backend.blockchain.provider().latest().map(Arc::new)?;

            let (public_key_x, public_key_y) = this.vrf_ctx.get_public_key_xy();
            let vrf_address = this.vrf_ctx.get_vrf_address(*pm_address);

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

            let vrf_calls = futures::executor::block_on(handle_vrf_calls(&outside_execution, chain_id, vrf_address, this.vrf_ctx.private_key, this.vrf_ctx.cache.clone()))?;

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
                resource_bounds: ResourceBoundsMapping::default(),
                tip: 0_u64,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                fee_data_availability_mode: DataAvailabilityMode::L1,
            };
            let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

            let signer = LocalWallet::from(SigningKey::from_secret_scalar(pm_private_key));
            let signature =
                futures::executor::block_on(signer.sign_hash(&tx_hash)).map_err(|e| anyhow!(e))?;
            tx.signature = vec![signature.r, signature.s];

            let tx = ExecutableTxWithHash::new(ExecutableTx::Invoke(InvokeTx::V3(tx)));
            let hash = this.pool.add_transaction(tx)?;

            Ok(InvokeTxResult::new(hash))
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

#[async_trait]
impl<EF: ExecutorFactory> CartridgeApiServer for CartridgeApi<EF> {
    async fn add_execute_outside_transaction(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
    ) -> RpcResult<InvokeTxResult> {
        Ok(self.execute_outside(address, outside_execution, signature).await?)
    }
}

/// Response from the Cartridge API to fetch the calldata for the constructor of the given
/// controller address.
#[derive(Debug, Deserialize)]
struct CartridgeAccountResponse {
    /// The address of the controller account.
    #[allow(unused)]
    address: Felt,
    /// The username of the controller account used as salt.
    #[allow(unused)]
    username: String,
    /// The calldata for the constructor of the given controller address, this is
    /// UDC calldata, already containing the class hash and the salt + the constructor arguments.
    calldata: Vec<Felt>,
}

/// Fetch the calldata for the constructor of the given controller address.
///
/// Returns `None` if the `address` is not associated with a Controller account.
async fn fetch_controller_constructor_calldata(
    cartridge_api_url: &Url,
    address: ContractAddress,
) -> anyhow::Result<Option<Vec<Felt>>> {
    debug!(target: "rpc::cartridge", %address, "Fetching Controller constructor calldata");
    let account_data_url = cartridge_api_url.join("/accounts/calldata")?;

    let body = serde_json::json!({
        "address": address
    });

    let client = reqwest::Client::new();
    let response = client
        .post(account_data_url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let response = response.text().await?;
    if response.contains("Address not found") {
        Ok(None)
    } else {
        let account = serde_json::from_str::<CartridgeAccountResponse>(&response)?;
        Ok(Some(account.calldata))
    }
}

/// Encodes the given calls into a vector of Felt values (New encoding, cairo 1),
/// since controller accounts are Cairo 1 contracts.
pub fn encode_calls(calls: Vec<Call>) -> Vec<Felt> {
    let mut execute_calldata: Vec<Felt> = vec![calls.len().into()];
    for call in calls {
        execute_calldata.push(call.to);
        execute_calldata.push(call.selector);

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
    cartridge_api_url: &Url,
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
            cartridge_api_url,
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
    cartridge_api_url: &Url,
    controller_address: ContractAddress,
    paymaster_address: ContractAddress,
    paymaster_private_key: Felt,
    chain_id: ChainId,
    paymaster_nonce: Felt,
) -> anyhow::Result<Option<ExecutableTxWithHash>> {
    if let Some(calldata) =
        fetch_controller_constructor_calldata(cartridge_api_url, controller_address).await?
    {
        let call = Call {
            to: DEFAULT_UDC_ADDRESS.into(),
            selector: selector!("deployContract"),
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
            resource_bounds: ResourceBoundsMapping::default(),
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
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
        CARTIDGE_VRF_CLASS_HASH,
        CARTIDGE_VRF_SALT,
        // from zero
        Felt::ZERO,
        // Calldata len
        Felt::from_hex_unchecked("0x3"),
        // owner
        paymaster_address.into(),
        // public key
        public_key_x,
        public_key_y,
    ];

    let call =
        Call { to: DEFAULT_UDC_ADDRESS.into(), selector: selector!("deployContract"), calldata };

    let mut tx = InvokeTxV3 {
        chain_id,
        tip: 0_u64,
        signature: vec![],
        paymaster_data: vec![],
        account_deployment_data: vec![],
        sender_address: paymaster_address,
        calldata: encode_calls(vec![call]),
        nonce: paymaster_nonce,
        resource_bounds: ResourceBoundsMapping::default(),
        nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
        fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
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

/// Computes a VRF proof for the given seed.
fn stark_vrf(seed: Felt, vrf_private_key: Felt) -> anyhow::Result<StarkVrfProof> {
    let private_key = vrf_private_key.to_string();
    let public_key = generate_public_key(private_key.parse().unwrap());

    let seed = vec![BaseField::from_str(&format!("{seed}")).unwrap()];

    let ecvrf = StarkVRF::new(public_key).unwrap();
    let proof = ecvrf.prove(&private_key.parse().unwrap(), seed.as_slice()).unwrap();
    let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed.as_slice());
    let rnd = ecvrf.proof_to_hash(&proof).unwrap();

    let beta = ecvrf.proof_to_hash(&proof).unwrap();

    debug!(target: "rpc::cartridge", seed = ?seed[0], random_value = %format(beta), "Computing VRF proof.");

    Ok(StarkVrfProof {
        gamma_x: format(proof.0.x),
        gamma_y: format(proof.0.y),
        c: format(proof.1),
        s: format(proof.2),
        sqrt_ratio: format(sqrt_ratio_hint),
        rnd: format(rnd),
    })
}

/// Formats the given value as a hexadecimal string.
fn format<T: std::fmt::Display>(v: T) -> String {
    let int = BigInt::from_str(&format!("{v}")).unwrap();
    format!("0x{}", int.to_str_radix(16))
}

/// Inspects the [`OutsideExecution`] to search for `request_random` call sent to the VRF contract
/// as the first call.
///
/// If it's a VRF call, the calls to submit randomness and assert consumption are returned.
/// Otherwise, an empty vector is returned.
///
/// In the current implementation, Katana doesn't store the cached nonces into the database, so any
/// restart of Katana would result in a reset of this nonce (hence predictible VRF).
async fn handle_vrf_calls(
    outside_execution: &OutsideExecution,
    chain_id: ChainId,
    vrf_address: ContractAddress,
    vrf_private_key: Felt,
    vrf_cache: Arc<RwLock<HashMap<Felt, Felt>>>,
) -> anyhow::Result<Vec<Call>> {
    let calls = match outside_execution {
        OutsideExecution::V2(v2) => &v2.calls,
        OutsideExecution::V3(v3) => &v3.calls,
    };

    if calls.is_empty() {
        anyhow::bail!("No calls in outside execution.");
    }

    let first_call = calls.first().unwrap();

    if first_call.selector != selector!("request_random") && first_call.to != (*vrf_address).into()
    {
        return Ok(vec![]);
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
        let nonce = *vrf_cache.read().unwrap().get(&contract_address).unwrap_or(&Felt::ZERO);
        vrf_cache.write().unwrap().insert(contract_address, nonce + Felt::ONE);
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

    let proof = stark_vrf(seed, vrf_private_key)?;

    let submit_random_call = Call {
        to: *vrf_address,
        selector: selector!("submit_random"),
        calldata: vec![
            seed,
            Felt::from_hex_unchecked(&proof.gamma_x),
            Felt::from_hex_unchecked(&proof.gamma_y),
            Felt::from_hex_unchecked(&proof.c),
            Felt::from_hex_unchecked(&proof.s),
            Felt::from_hex_unchecked(&proof.sqrt_ratio),
        ],
    };

    let assert_consumed_call =
        Call { to: *vrf_address, selector: selector!("assert_consumed"), calldata: vec![seed] };

    Ok(vec![submit_random_call, assert_consumed_call])
}
