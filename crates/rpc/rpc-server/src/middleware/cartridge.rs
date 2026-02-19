use std::borrow::Cow;
use std::collections::HashSet;
use std::future::Future;
use std::iter::once;

use cainome_cairo_serde::CairoSerde;
use cartridge::utils::find_request_rand_call;
use cartridge::CartridgeApiClient;
use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::http_client::HttpClient;
use jsonrpsee::types::Request;
use jsonrpsee::MethodResponse;
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_paymaster::api::PaymasterApiClient;
use katana_pool::{TransactionPool, TxPool};
use katana_pool_api::PoolError;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::chain::ChainId;
use katana_primitives::contract::Nonce;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::hash::{Poseidon, StarkHash};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::{ProviderFactory, ProviderRO, ProviderRO};
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::outside_execution::{Call as OutsideExecutionCall, OutsideExecution};
use katana_rpc_types::{BroadcastedInvokeTx, FeeEstimate};
use layer::PaymasterLayer;
use serde::Deserialize;
use serde_json::to_string;
use starknet::core::types::SimulationFlagForEstimateFee;
use starknet::macros::selector;
use starknet::providers::jsonrpc::JsonRpcResponse;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use starknet_types_core::hash::Pedersen;
use tracing::{debug, trace, trace};
use url::Url;

use super::ControllerDeployment;
use crate::cartridge::{
    build_execute_from_outside_call_from_vrf_result, VrfService, VrfServiceConfig,
};
use crate::utils::{self, encode_calls};

pub type PaymasterResult<T> = Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cartridge client error: {0}")]
    Client(#[from] crate::client::Error),

    #[error("provider error: {0}")]
    Provider(#[from] katana_provider::api::ProviderError),

    #[error("paymaster not found")]
    PaymasterNotFound(ContractAddress),

    #[error("VRF error: {0}")]
    Vrf(String),

    #[error("failed to sign with paymaster: {0}")]
    SigningError(String),

    #[error("failed to add deploy controller transaction to the pool: {0}")]
    FailedToAddTransaction(#[from] PoolError),
}

impl From<VrfClientError> for Error {
    fn from(e: VrfClientError) -> Self {
        Error::Vrf(e.to_string())
    }
}

#[derive(Debug)]
pub struct ControllerDeployment {
    chain_id: ChainId,
    cartridge_api: CartridgeApiClient,
    paymaster_client: HttpClient,
    vrf_service: Option<VrfService>,
}

impl ControllerDeployment {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: ChainId,
        cartridge_api: CartridgeApiClient,
        paymaster_client: HttpClient,
        vrf: Option<VrfServiceConfig>,
    ) -> Self {
        Self {
            chain_id,
            cartridge_api,
            paymaster_client,
            vrf_service: config.vrf.map(VrfService::new),
        }
    }

    /// Handle the intercept of the 'starknet_estimateFee' end point.
    pub async fn handle_estimate_fee_inner(
        &self,
        _block_id: katana_rpc_types::BlockIdOrTag,
        transactions: &Vec<BroadcastedTx>,
    ) -> PaymasterResult<Option<Vec<BroadcastedTx>>> {
        let mut deployed_controllers: HashSet<ContractAddress> = HashSet::new();
        let mut new_transactions = Vec::new();
        let mut updated_transactions = Vec::new();
        let mut has_updated_transactions = false;

        let mut paymaster_nonce = self.get_paymaster_nonce()?;

        // Process the transactions to check if some controller needs to be deployed and
        // if some VRF calls have to be inserted between the original calls.
        for tx in transactions {
            let address = match &tx {
                BroadcastedTx::Invoke(invoke_tx) => {
                    // Try to inject VRF calls into invoke transactions.
                    let updated_tx = match self.decode_calls(&invoke_tx.calldata) {
                        Some(calls) => match self.get_vrf_calls(&calls).await? {
                            Some(vrf_calls) => {
                                // has_updated_transactions = true;

                                // let [submit_call, assert_call] = vrf_calls;
                                // let calls = once(submit_call)
                                //     .chain(calls.iter().cloned())
                                //     .chain(once(assert_call))
                                //     .collect::<Vec<_>>();

                                // BroadcastedTx::Invoke(BroadcastedInvokeTx {
                                //     sender_address: invoke_tx.sender_address,
                                //     calldata: self.encode_calls(&calls),
                                //     signature: invoke_tx.signature.clone(),
                                //     nonce: invoke_tx.nonce,
                                //     tip: invoke_tx.tip,
                                //     paymaster_data: invoke_tx.paymaster_data.clone(),
                                //     resource_bounds: invoke_tx.resource_bounds.clone(),
                                //     nonce_data_availability_mode: invoke_tx
                                //         .nonce_data_availability_mode,
                                //     fee_data_availability_mode: invoke_tx
                                //         .fee_data_availability_mode,
                                //     account_deployment_data: invoke_tx
                                //         .account_deployment_data
                                //         .clone(),
                                //     is_query: invoke_tx.is_query,
                                // })

                                todo!()
                            }

                            None => tx.clone(),
                        },

                        None => tx.clone(),
                    };

                    updated_transactions.push(updated_tx);
                    invoke_tx.sender_address
                }
                BroadcastedTx::Declare(declare_tx) => {
                    updated_transactions.push(tx.clone());
                    declare_tx.sender_address
                }
                _ => {
                    updated_transactions.push(tx.clone());
                    continue;
                }
            };

            // If the address has already been processed in this txs batch, just skip.
            if deployed_controllers.contains(&address) {
                continue;
            }

            let tx_opt = self.craft_controller_deploy_tx(address, paymaster_nonce).await?;
            if let Some(tx) = tx_opt {
                deployed_controllers.insert(address);

                let tx_hash = self
                    .pool
                    .add_transaction(ExecutableTxWithHash::new(tx.clone()))
                    .await
                    .map_err(Error::FailedToAddTransaction)?;

                new_transactions.push(self.executable_tx_to_broadcasted(tx));

                trace!(
                    target: "cartridge",
                    controller = %address,
                    tx_hash = format!("{tx_hash:#x}"),
                    "Estimate fee: Controller deploy transaction submitted");

                paymaster_nonce += Nonce::ONE;
            }
        }

        if !new_transactions.is_empty() || has_updated_transactions {
            new_transactions.extend(updated_transactions.iter().cloned());
            return Ok(Some(new_transactions));
        }

        Ok(None)
    }

    /// Returns a [`Layer`](tower::Layer) implementation of [`Paymaster`].
    ///
    /// This allows the paymaster to be used as a middleware in Katana RPC stack.
    pub fn layer(self) -> PaymasterLayer<PF> {
        PaymasterLayer { paymaster: self }
    }

    /// Crafts a deploy controller transaction for a cartridge controller.
    ///
    /// Returns None if the provided `controller_address` is not registered in the Cartridge API,
    /// or if it has already been deployed.
    async fn craft_controller_deploy_tx(
        &self,
        address: ContractAddress,
        paymaster_nonce: Felt,
    ) -> PaymasterResult<Option<ExecutableTx>> {
        // If the address is not a controller, just ignore the tx.
        let controller_calldata = match self.get_controller_ctor_calldata(address).await? {
            Some(calldata) => calldata,
            None => return Ok(None),
        };

        // Check if the address has already been deployed using the provider directly.
        let state = self.provider.provider().latest()?;
        if state.class_hash_of_contract(address)?.is_some() {
            return Ok(None);
        }

        // Create a Controller deploy transaction against the latest state of the network.
        debug!(target: "cartridge", controller = %address, "Crafting controller deploy transaction");

        let call = OutsideExecutionCall {
            to: DEFAULT_UDC_ADDRESS,
            selector: selector!("deployContract"),
            calldata: controller_calldata,
        };

        let mut tx = InvokeTxV3 {
            nonce: paymaster_nonce,
            chain_id: self.chain_id,
            tip: 0_u64,
            signature: Vec::new(),
            sender_address: self.paymaster_address,
            paymaster_data: Vec::new(),
            calldata: encode_calls(vec![call]),
            account_deployment_data: Vec::new(),
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
        };

        let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

        let signer = LocalWallet::from(self.paymaster_key.clone());
        let signature = signer.sign_hash(&tx_hash).await.unwrap();
        tx.signature = vec![signature.r, signature.s];

        let tx = ExecutableTx::Invoke(InvokeTx::V3(tx));

        Ok(Some(tx))
    }

    /// Get the constructor calldata for a controller account or None if the address is not a
    /// controller.
    async fn get_controller_ctor_calldata(
        &self,
        address: ContractAddress,
    ) -> PaymasterResult<Option<Vec<Felt>>> {
        let result = self.cartridge_api.get_account_calldata(address).await?;
        Ok(result.map(|r| r.constructor_calldata))
    }

    fn decode_calls(&self, calldata: &[Felt]) -> Option<Vec<OutsideExecutionCall>> {
        Vec::<OutsideExecutionCall>::cairo_deserialize(calldata, 0).ok()
    }

    fn encode_calls(&self, calls: &Vec<OutsideExecutionCall>) -> Vec<Felt> {
        Vec::<OutsideExecutionCall>::cairo_serialize(calls)
    }

    fn get_calls_from_outside_execution(
        &self,
        outside_execution: &OutsideExecution,
    ) -> Vec<OutsideExecutionCall> {
        match outside_execution {
            OutsideExecution::V2(v2) => v2.calls.clone(),
            OutsideExecution::V3(v3) => v3.calls.clone(),
        }
    }

    /// Get the VRF calls for a given set of decoded invoke transaction calls.
    ///
    /// Uses the external VRF server via [`VrfClient::proof`] to generate VRF proofs.
    ///
    /// Returns None if the calls do not contain any 'request_random' VRF call
    /// targeting the VRF account.
    async fn get_vrf_calls(
        &self,
        calls: &[OutsideExecutionCall],
    ) -> PaymasterResult<Option<[OutsideExecutionCall; 2]>> {
        if calls.is_empty() {
            return Ok(None);
        }

        if let Some(vrf_service) = &self.vrf_service {
            if let Some((rand_call, pos)) = find_request_rand_call(calls) {
                if pos + 1 >= calls_len {
                    return Err(Error::Vrf(format!(
                        "request_random call must be followed by another call",
                    )));
                }

                if rand_call.to != vrf_service.account_address() {
                    return Err(Error::Vrf(format!(
                        "request_random call must target the vrf account",
                    )));
                }

                let result = vrf_service
                    .outside_execution(address, &outside_execution, &signature, self.chain_id)
                    .await
                    .map_err(|e| Error::Vrf(e.to_string()))?;

                user_address = result.address;
                execute_from_outside_call =
                    build_execute_from_outside_call_from_vrf_result(&result);
            }
        }

        // // If request_random targeting the VRF account is the only call, just ignore it
        // // as the generated random value will not be consumed.
        // if calls.len() == 1 {
        //     return Ok(None);
        // }

        // let caller = first_call.calldata[0];
        // let salt_or_nonce_selector = first_call.calldata[1];
        // // Salt or nonce being the salt for the `Salt` variant, and the contract address for the
        // // `Nonce` variant.
        // let salt_or_nonce = first_call.calldata[2];

        // let source = if salt_or_nonce_selector == Felt::ZERO {
        //     let contract_address = salt_or_nonce;
        //     let state = self.provider.provider().latest()?;

        //     let key = Pedersen::hash(&selector!("VrfProvider_nonces"), &contract_address);
        //     state.storage(self.vrf_account_address, key)?.unwrap_or_default()
        // } else if salt_or_nonce_selector == Felt::ONE {
        //     salt_or_nonce
        // } else {
        //     return Err(Error::Vrf(format!(
        //         "Invalid salt or nonce for VRF request, expecting 0 or 1, got \
        //          {salt_or_nonce_selector}"
        //     )));
        // };

        // let seed = Poseidon::hash_array(&[source, caller, self.chain_id.id()]);

        // // Use external VRF server to generate the proof.
        // let proof = self.vrf_client.proof(vec![seed.to_hex_string()]).await?;

        // let submit_random_call = OutsideExecutionCall {
        //     to: self.vrf_account_address,
        //     selector: selector!("submit_random"),
        //     calldata: vec![seed, proof.gamma_x, proof.gamma_y, proof.c, proof.s,
        // proof.sqrt_ratio], };

        // let assert_consumed_call = OutsideExecutionCall {
        //     selector: selector!("assert_consumed"),
        //     to: self.vrf_account_address,
        //     calldata: vec![seed],
        // };

        // Ok(Some([submit_random_call, assert_consumed_call]))

        todo!()
    }

    /// Get the nonce of the paymaster account.
    ///
    /// Checks the pool nonce first (for pending state), then falls back to the provider.
    fn get_paymaster_nonce(&self) -> PaymasterResult<Felt> {
        // Check pool nonce first for the most up-to-date value.
        if let Some(nonce) = self.pool.get_nonce(self.paymaster_address) {
            return Ok(nonce);
        }

        // Fallback to state from provider.
        let state = self.provider.provider().latest()?;
        match state.nonce(self.paymaster_address)? {
            Some(nonce) => Ok(nonce),
            None => Err(Error::PaymasterNotFound(self.paymaster_address)),
        }
    }
}

impl Clone for ControllerDeployment {
    fn clone(&self) -> Self {
        Self {
            chain_id: self.chain_id,
            vrf_service: self.vrf_service.clone(),
            cartridge_api: self.cartridge_api.clone(),
            paymaster_client: self.paymaster_client.clone(),
        }
    }
}

#[derive(Deserialize)]
struct EstimateFeeParams {
    #[serde(alias = "request")]
    txs: Vec<BroadcastedTx>,
    #[serde(alias = "simulationFlags")]
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    #[serde(alias = "blockId")]
    block_id: BlockIdOrTag,
}

#[derive(Debug, Clone)]
pub struct PaymasterLayer {
    pub(crate) paymaster: ControllerDeployment,
}

impl<S> tower::Layer<S> for PaymasterLayer {
    type Service = PaymasterService<S>;

    fn layer(&self, service: S) -> Self::Service {
        PaymasterService { service, paymaster: self.paymaster.clone() }
    }
}

#[derive(Debug)]
pub struct PaymasterService<S> {
    service: S,
    paymaster: ControllerDeployment,
}

impl<S> PaymasterService<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send + Sync + Clone + 'static,
{
    /// Extract estimate_fee parameters from the request.
    fn parse_estimate_fee_params(request: &Request<'_>) -> Option<EstimateFeeParams> {
        let params = request.params();

        if params.is_object() {
            match params.parse() {
                Ok(p) => Some(p),
                Err(..) => {
                    debug!(target: "cartridge", "Failed to parse estimate fee params.");
                    None
                }
            }
        } else {
            let mut seq = params.sequence();

            let txs_result: Result<Vec<BroadcastedTx>, _> = seq.next();
            let simulation_flags_result: Result<Vec<SimulationFlagForEstimateFee>, _> = seq.next();
            let block_id_result: Result<BlockIdOrTag, _> = seq.next();

            match (txs_result, simulation_flags_result, block_id_result) {
                (Ok(txs), Ok(simulation_flags), Ok(block_id)) => {
                    Some(EstimateFeeParams { txs, simulation_flags, block_id })
                }
                _ => {
                    debug!(target: "cartridge", "Failed to parse estimate fee params.");
                    None
                }
            }
        }
    }

    /// Build a new estimate fee request with the updated transactions.
    fn build_new_estimate_fee_request<'a>(
        request: &Request<'a>,
        params: &EstimateFeeParams,
        updated_txs: &Vec<BroadcastedTx>,
    ) -> Request<'a> {
        let mut new_request = request.clone();

        let mut new_params = jsonrpsee::core::params::ArrayParams::new();
        new_params.insert(updated_txs).unwrap();
        new_params.insert(params.simulation_flags.clone()).unwrap();
        new_params.insert(params.block_id).unwrap();

        let new_params = new_params.to_rpc_params().unwrap();
        new_request.params = new_params.map(Cow::Owned);
        new_request
    }

    // <--- TODO: this function should be removed once estimateFee will return 0 fees
    // when --dev.no-fee is used.
    fn build_no_fee_response(request: &Request<'_>, count: usize) -> MethodResponse {
        let estimate_fees = vec![
            FeeEstimate {
                l1_gas_consumed: 0,
                l1_gas_price: 0,
                l2_gas_consumed: 0,
                l2_gas_price: 0,
                l1_data_gas_consumed: 0,
                l1_data_gas_price: 0,
                overall_fee: 0
            };
            count
        ];

        MethodResponse::response(
            request.id().clone(),
            jsonrpsee::ResponsePayload::success(estimate_fees),
            usize::MAX,
        )
    }
    // end of the no-fee response

    async fn handle_estimate_fee<'a>(
        service: S,
        paymaster: ControllerDeployment<PF>,
        request: Request<'a>,
    ) -> S::MethodResponse {
        if let Some(params) = Self::parse_estimate_fee_params(&request) {
            let updated_txs = paymaster
                .handle_estimate_fee_inner(params.block_id, &params.txs)
                .await
                .unwrap_or_default();

            if let Some(updated_txs) = updated_txs {
                let new_request =
                    Self::build_new_estimate_fee_request(&request, &params, &updated_txs);

                let response = service.call(new_request).await;

                // if `handle_estimate_fees` has added some new transactions at the
                // beginning of updated_txs, we have to remove
                // extras results from estimate_fees to be
                // sure to return the same number of result than the number
                // of transactions in the request.
                let nb_of_txs = params.txs.len();
                let nb_of_extra_txs = updated_txs.len() - nb_of_txs;

                if response.is_success() && nb_of_extra_txs > 0 {
                    if let Ok(JsonRpcResponse::Success { result: mut estimate_fees, .. }) =
                        serde_json::from_str::<JsonRpcResponse<Vec<FeeEstimate>>>(
                            response.to_json().get(),
                        )
                    {
                        if estimate_fees.len() >= nb_of_extra_txs {
                            estimate_fees.drain(0..nb_of_extra_txs);
                        }

                        trace!(
                            target: "cartridge",
                            nb_of_extra_txs = nb_of_extra_txs,
                                nb_of_estimate_fees = estimate_fees.len(),
                            "Removing extra transactions from estimate fees response",
                        );

                        // TODO: restore the real response
                        return Self::build_no_fee_response(&request, nb_of_txs);
                    }
                }

                trace!(target: "cartridge", "Estimate fee endpoint original response returned");

                // TODO: restore the real response
                return Self::build_no_fee_response(&request, nb_of_txs);
            }
        }

        trace!(target: "cartridge", "Estimate fee endpoint called with the original transaction");
        service.call(request).await
    }
}

impl<S> RpcServiceT for PaymasterService<S>
where
    S: RpcServiceT<
            MethodResponse = MethodResponse,
            BatchResponse = MethodResponse,
            NotificationResponse = MethodResponse,
        > + Send
        + Sync
        + Clone
        + 'static,
{
    type MethodResponse = S::MethodResponse;
    type BatchResponse = S::BatchResponse;
    type NotificationResponse = S::NotificationResponse;

    fn call<'a>(
        &self,
        request: Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let service = self.service.clone();
        let paymaster = self.paymaster.clone();

        async move {
            if request.method_name() == "starknet_estimateFee" {
                Self::handle_estimate_fee(service, paymaster, request).await
            } else {
                service.call(request).await
            }
        }
    }

    fn batch<'a>(
        &self,
        requests: Batch<'a>,
    ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        self.service.batch(requests)
    }

    fn notification<'a>(
        &self,
        n: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.service.notification(n)
    }
}

impl<S: Clone> Clone for PaymasterService<S> {
    fn clone(&self) -> Self {
        Self { service: self.service.clone(), paymaster: self.paymaster.clone() }
    }
}
