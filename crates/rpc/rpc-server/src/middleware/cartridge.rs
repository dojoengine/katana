use std::borrow::Cow;
use std::collections::HashSet;
use std::future::Future;

use cartridge::vrf::VrfClientError;
use cartridge::CartridgeApiClient;
use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::http_client::HttpClient;
use jsonrpsee::types::{ErrorObjectOwned, Request, Response, ResponsePayload};
use jsonrpsee::{rpc_params, MethodResponse};
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_pool::api::{PoolError, TransactionPool};
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::execution::Call;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::{ContractAddress, Felt};
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::error::cartridge::CartridgeApiError;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::broadcasted::{BroadcastedTx, BroadcastedTxWithChainId};
use katana_rpc_types::{BroadcastedInvokeTx, FeeEstimate, FeeSource, OutsideExecution};
use serde::Deserialize;
use starknet::core::types::SimulationFlagForEstimateFee;
use starknet::macros::selector;
use starknet::signers::local_wallet::SignError;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tracing::{debug, trace};

use crate::cartridge::{encode_calls, VrfService};
use crate::starknet::{PendingBlockProvider, StarknetApi};

#[derive(Debug)]
pub struct ControllerDeploymentLayer<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    starknet: StarknetApi<Pool, PP, PF>,
    cartridge_api: CartridgeApiClient,
    paymaster_client: HttpClient,
    deployer_address: ContractAddress,
    deployer_private_key: SigningKey,
    vrf_service: Option<VrfService>,
}

#[derive(Debug)]
pub struct ControllerDeploymentService<S, Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    starknet: StarknetApi<Pool, PP, PF>,
    cartridge_api: CartridgeApiClient,
    paymaster_client: HttpClient,
    deployer_address: ContractAddress,
    deployer_private_key: SigningKey,
    vrf_service: Option<VrfService>,
    service: S,
}

impl<S, Pool, PoolTx, PP, PF> ControllerDeploymentService<S, Pool, PP, PF>
where
    S: RpcServiceT<MethodResponse = MethodResponse>,
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    // if `handle_estimate_fees` has added some new transactions at the
    // beginning of updated_txs, we have to remove
    // extras results from estimate_fees to be
    // sure to return the same number of result than the number
    // of transactions in the request.
    async fn handle_estimate_fee<'a>(
        &self,
        params: EstimateFeeParams,
        request: Request<'a>,
    ) -> S::MethodResponse {
        match self.handle_estimate_fee_inner(params, request).await {
            Ok(response) => response,
            Err(err) => MethodResponse::error(request.id().clone(), ErrorObjectOwned::from(err)),
        }
    }

    async fn handle_execute_outside<'a>(
        &self,
        params: AddExecuteOutsideParams,
        request: Request<'a>,
    ) -> S::MethodResponse {
        if let Err(err) = self.handle_execute_outside_inner(params).await {
            MethodResponse::error(request.id().clone(), ErrorObjectOwned::from(err))
        } else {
            self.service.call(request).await
        }
    }

    async fn handle_estimate_fee_inner<'a>(
        &self,
        params: EstimateFeeParams,
        request: Request<'a>,
    ) -> Result<S::MethodResponse, CartridgeApiError> {
        let EstimateFeeParams { block_id, simulation_flags, transactions } = params;

        let mut undeployed_addresses: Vec<ContractAddress> = Vec::new();

        // iterate thru all txs and deploy any undeployed contract (if they are a Controller)
        for tx in &transactions {
            let address = match tx {
                BroadcastedTx::Invoke(tx) => tx.sender_address,
                BroadcastedTx::Declare(tx) => tx.sender_address,
                _ => continue,
            };

            undeployed_addresses.push(address);
        }

        let deployer_nonce = self.starknet.nonce_at(block_id, self.deployer_address).await.unwrap();
        let deploy_controller_txs =
            self.get_controller_deployment_txs(undeployed_addresses, deployer_nonce).await.unwrap();

        // no Controller to deploy, simply forward the request
        if deploy_controller_txs.is_empty() {
            return Ok(self.service.call(request).await);
        }

        let original_txs_count = transactions.len();
        let deploy_controller_txs_count = deploy_controller_txs.len();

        let new_txs = [deploy_controller_txs, transactions].concat();
        let new_txs_count = new_txs.len();

        // craft a new estimate fee request with the deploy Controller txs included
        let new_request = {
            let params = rpc_params!(new_txs, simulation_flags, block_id);
            let params = params.to_rpc_params().unwrap();

            let mut new_request = request.clone();
            new_request.params = params.map(Cow::Owned);

            new_request
        };

        let response = self.service.call(new_request).await;

        let res = response.as_json().get();
        let mut res = serde_json::from_str::<Response<Vec<FeeEstimate>>>(res).unwrap();

        match res.payload {
            ResponsePayload::Success(mut estimates) => {
                assert_eq!(estimates.len(), new_txs_count);
                estimates.to_mut().drain(0..deploy_controller_txs_count);
                Ok(build_no_fee_response(&request, original_txs_count))
            }

            ResponsePayload::Error(..) => Ok(response),
        }
    }

    async fn handle_execute_outside_inner(
        &self,
        params: AddExecuteOutsideParams,
    ) -> Result<(), CartridgeApiError> {
        let address = params.address;
        let block_id = BlockIdOrTag::PreConfirmed;

        // check if the address has already been deployed.
        let is_deployed = match self.starknet.class_hash_at_address(block_id, address).await {
            Ok(..) => true,
            Err(StarknetApiError::ContractNotFound) => false,
            Err(e) => {
                return Err(CartridgeApiError::ControllerDeployment {
                    reason: format!("failed to check Controller deployment status: {e}"),
                });
            }
        };

        if is_deployed {
            return Ok(());
        }

        let result = self.starknet.nonce_at(block_id, self.deployer_address).await;
        let nonce = match result {
            Ok(nonce) => nonce,
            Err(e) => {
                return Err(CartridgeApiError::ControllerDeployment {
                    reason: format!("failed to get deployer nonce: {e}"),
                });
            }
        };

        let result = self.get_controller_deployment_tx(address, nonce).await;
        let deploy_tx = match result {
            Ok(tx) => tx,
            Err(e) => {
                return Err(CartridgeApiError::ControllerDeployment { reason: e.to_string() });
            }
        };

        // None means the address is not of a Controller
        if let Some(tx) = deploy_tx {
            if let Err(e) = self.starknet.add_invoke_tx(tx).await {
                return Err(CartridgeApiError::ControllerDeployment {
                    reason: format!("failed to submit deployment tx: {e}"),
                });
            }
        }

        Ok(())
    }

    async fn get_controller_deployment_txs(
        &self,
        controller_addreses: Vec<ContractAddress>,
        initial_nonce: Nonce,
    ) -> Result<Vec<BroadcastedTx>, Error> {
        let mut deploy_transactions: Vec<BroadcastedTx> = Vec::new();
        let mut processed_addresses: Vec<ContractAddress> = Vec::new();

        let mut deployer_nonce = initial_nonce;

        for address in controller_addreses {
            // If the address has already been processed in this txs batch, just skip.
            if processed_addresses.contains(&address) {
                continue;
            }

            let deploy_tx = self.get_controller_deployment_tx(address, deployer_nonce).await?;

            // None means the address is not a Controller
            if let Some(tx) = deploy_tx {
                deployer_nonce += Nonce::ONE;
                processed_addresses.push(address);
                deploy_transactions.push(BroadcastedTx::Invoke(tx));
            }
        }

        Ok(deploy_transactions)
    }

    async fn get_controller_deployment_tx(
        &self,
        address: ContractAddress,
        paymaster_nonce: Nonce,
    ) -> Result<Option<BroadcastedInvokeTx>, Error> {
        let Some(ctor_calldata) = self.cartridge_api.get_account_calldata(address).await? else {
            // this means no controller with the given address
            return Ok(None);
        };

        let call = Call {
            contract_address: DEFAULT_UDC_ADDRESS,
            calldata: ctor_calldata.constructor_calldata,
            entry_point_selector: selector!("deployContract"),
        };

        let mut tx = BroadcastedInvokeTx {
            sender_address: self.deployer_address,
            calldata: encode_calls(vec![call]),
            signature: Vec::new(),
            nonce: paymaster_nonce,
            paymaster_data: Vec::new(),
            tip: 0u64.into(),
            account_deployment_data: Vec::new(),
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        };

        let signature = {
            let chain = self.starknet.chain_id();
            let tx = BroadcastedTx::Invoke(tx.clone());
            let tx = BroadcastedTxWithChainId { tx, chain: chain.into() };

            let signer = LocalWallet::from(self.deployer_private_key.clone());

            let tx_hash = tx.calculate_hash();
            signer.sign_hash(&tx_hash).await.map_err(Error::SigningError)?
        };

        tx.signature = vec![signature.r, signature.s];

        Ok(Some(tx))
    }
}

impl<S, Pool, PoolTx, PP, PF> RpcServiceT for ControllerDeploymentService<S, Pool, PP, PF>
where
    S: RpcServiceT + Send + Sync + Clone + 'static,
    S: RpcServiceT<MethodResponse = MethodResponse>,
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    type MethodResponse = S::MethodResponse;
    type BatchResponse = S::BatchResponse;
    type NotificationResponse = S::NotificationResponse;

    fn call<'a>(
        &self,
        request: Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let this = self.clone();

        async move {
            let method = request.method_name();

            match method {
                "starknet_estimateFee" => {
                    trace!(%method, "Intercepting JSON-RPC method.");
                    if let Some(params) = parse_estimate_fee_params(&request) {
                        return this.handle_estimate_fee(params, request).await;
                    }
                }

                "addExecuteOutsideTransaction" | "addExecuteFromOutside" => {
                    trace!(%method, "Intercepting JSON-RPC method.");
                    if let Some(params) = parse_execute_outside_params(&request) {
                        return this.handle_execute_outside(params, request).await;
                    }
                }

                _ => {}
            }

            this.service.call(request).await
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

impl<S, Pool, PP, PF> Clone for ControllerDeploymentService<S, Pool, PP, PF>
where
    S: Clone,
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            starknet: self.starknet.clone(),
            vrf_service: self.vrf_service.clone(),
            cartridge_api: self.cartridge_api.clone(),
            paymaster_client: self.paymaster_client.clone(),
            deployer_address: self.deployer_address.clone(),
            deployer_private_key: self.deployer_private_key.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cartridge api error: {0}")]
    Client(#[from] cartridge::api::Error),

    #[error("provider error: {0}")]
    Provider(#[from] katana_provider::api::ProviderError),

    #[error("paymaster not found")]
    PaymasterNotFound(ContractAddress),

    #[error("VRF error: {0}")]
    Vrf(String),

    #[error("failed to sign with paymaster: {0}")]
    SigningError(SignError),

    #[error("failed to add deploy controller transaction to the pool: {0}")]
    FailedToAddTransaction(#[from] PoolError),
}

impl From<VrfClientError> for Error {
    fn from(e: VrfClientError) -> Self {
        Error::Vrf(e.to_string())
    }
}

#[derive(Deserialize)]
struct AddExecuteOutsideParams {
    address: ContractAddress,
    outside_execution: OutsideExecution,
    signature: Vec<Felt>,
    fee_source: Option<FeeSource>,
}

#[derive(Deserialize)]
struct EstimateFeeParams {
    #[serde(alias = "request")]
    transactions: Vec<BroadcastedTx>,
    #[serde(alias = "simulationFlags")]
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    #[serde(alias = "blockId")]
    block_id: BlockIdOrTag,
}

fn parse_execute_outside_params(request: &Request<'_>) -> Option<AddExecuteOutsideParams> {
    let params = request.params();

    if params.is_object() {
        match params.parse() {
            Ok(p) => Some(p),
            Err(..) => {
                debug!(target: "cartridge", "Failed to parse execute outside params.");
                None
            }
        }
    } else {
        let mut seq = params.sequence();

        let address: Result<ContractAddress, _> = seq.next();
        let outside_execution: Result<OutsideExecution, _> = seq.next();
        let signature: Result<Vec<Felt>, _> = seq.next();
        let fee_source: Result<Option<FeeSource>, _> = seq.next();

        match (address, outside_execution, signature) {
            (Ok(address), Ok(outside_execution), Ok(signature)) => Some(AddExecuteOutsideParams {
                address,
                outside_execution,
                signature,
                fee_source: fee_source.ok().flatten(),
            }),
            _ => {
                debug!(target: "cartridge", "Failed to parse execute outside params.");
                None
            }
        }
    }
}

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
                Some(EstimateFeeParams { transactions: txs, simulation_flags, block_id })
            }
            _ => {
                debug!(target: "cartridge", "Failed to parse estimate fee params.");
                None
            }
        }
    }
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
