use std::borrow::Cow;
use std::collections::HashSet;
use std::future::Future;

use cartridge::CartridgeApiClient;
use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::{ErrorObjectOwned, Request, Response, ResponsePayload};
use jsonrpsee::{rpc_params, MethodResponse};
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_pool::api::TransactionPool;
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
use serde::de::DeserializeOwned;
use serde::Deserialize;
use starknet::core::types::SimulationFlagForEstimateFee;
use starknet::macros::selector;
use starknet::signers::local_wallet::SignError;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tower::Layer;
use tracing::{debug, trace};

use crate::cartridge::encode_calls;
use crate::starknet::{PendingBlockProvider, StarknetApi};

const STARKNET_ESTIMATE_FEE: &str = "starknet_estimateFee";
const CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE: &str = "cartridge_addExecuteFromOutside";
const CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE_TX: &str = "cartridge_addExecuteOutsideTransaction";

#[derive(Debug, Clone)]
struct ControllerDeploymentContext<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    starknet: StarknetApi<Pool, PP, PF>,
    cartridge_api: CartridgeApiClient,
    deployer_address: ContractAddress,
    deployer_private_key: SigningKey,
}

#[derive(Debug, Clone)]
pub struct ControllerDeploymentLayer<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    context: ControllerDeploymentContext<Pool, PP, PF>,
}

impl<Pool, PP, PF> ControllerDeploymentLayer<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    pub fn new(
        starknet: StarknetApi<Pool, PP, PF>,
        cartridge_api: CartridgeApiClient,
        deployer_address: ContractAddress,
        deployer_private_key: SigningKey,
    ) -> Self {
        let context = ControllerDeploymentContext {
            starknet,
            cartridge_api,
            deployer_address,
            deployer_private_key,
        };

        Self { context }
    }
}

impl<S, Pool, PoolTx, PP, PF> Layer<S> for ControllerDeploymentLayer<Pool, PP, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    type Service = ControllerDeploymentService<S, Pool, PP, PF>;

    fn layer(&self, inner: S) -> Self::Service {
        ControllerDeploymentService { context: self.context.clone(), service: inner }
    }
}

#[derive(Debug, Clone)]
pub struct ControllerDeploymentService<S, Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    context: ControllerDeploymentContext<Pool, PP, PF>,
    service: S,
}

impl<S, Pool, PoolTx, PP, PF> ControllerDeploymentService<S, Pool, PP, PF>
where
    S: RpcServiceT + Send + Sync + Clone + 'static,
    S: RpcServiceT<MethodResponse = MethodResponse>,
    Pool: TransactionPool<Transaction = PoolTx> + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    fn controller_deployment_error(reason: impl Into<String>) -> CartridgeApiError {
        CartridgeApiError::ControllerDeployment { reason: reason.into() }
    }

    fn estimate_fee_candidate_addresses(transactions: &[BroadcastedTx]) -> Vec<ContractAddress> {
        transactions
            .iter()
            .filter_map(|tx| match tx {
                BroadcastedTx::Invoke(tx) => Some(tx.sender_address),
                BroadcastedTx::Declare(tx) => Some(tx.sender_address),
                _ => None,
            })
            .collect()
    }

    fn build_estimate_fee_request<'a>(
        request: &Request<'a>,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockIdOrTag,
    ) -> Result<Request<'a>, CartridgeApiError> {
        let params = rpc_params!(transactions, simulation_flags, block_id);
        let params = params.to_rpc_params().map_err(|err| {
            Self::controller_deployment_error(format!(
                "failed to serialize augmented estimateFee params: {err}"
            ))
        })?;

        let mut new_request = request.clone();
        new_request.params = params.map(Cow::Owned);

        Ok(new_request)
    }

    // If deployment txs are added, return the no-fee estimates for the original requests only.
    async fn starknet_estimate_fee<'a>(
        &self,
        params: EstimateFeeParams,
        request: Request<'a>,
    ) -> S::MethodResponse {
        let request_id = request.id().clone();
        match self.starknet_estimate_fee_inner(params, request).await {
            Ok(response) => response,
            Err(err) => MethodResponse::error(request_id, ErrorObjectOwned::from(err)),
        }
    }

    async fn cartridge_add_execute_from_outside<'a>(
        &self,
        params: AddExecuteOutsideParams,
        request: Request<'a>,
    ) -> S::MethodResponse {
        if let Err(err) = self.cartridge_add_execute_from_outside_inner(params).await {
            MethodResponse::error(request.id().clone(), ErrorObjectOwned::from(err))
        } else {
            self.service.call(request).await
        }
    }

    async fn starknet_estimate_fee_inner<'a>(
        &self,
        params: EstimateFeeParams,
        request: Request<'a>,
    ) -> Result<S::MethodResponse, CartridgeApiError> {
        let EstimateFeeParams { block_id, simulation_flags, transactions } = params;
        let candidate_addresses = Self::estimate_fee_candidate_addresses(&transactions);

        let deployer_nonce = self
            .context
            .starknet
            .nonce_at(block_id, self.context.deployer_address)
            .await
            .map_err(|err| {
                Self::controller_deployment_error(format!("failed to get deployer nonce: {err}"))
            })?;
        let deploy_controller_txs = self
            .get_controller_deployment_txs(candidate_addresses, deployer_nonce)
            .await
            .map_err(|err| Self::controller_deployment_error(err.to_string()))?;

        // no Controller to deploy, simply forward the request
        if deploy_controller_txs.is_empty() {
            return Ok(self.service.call(request).await);
        }

        let original_txs_count = transactions.len();
        let new_txs = [deploy_controller_txs, transactions].concat();
        let new_txs_count = new_txs.len();
        let new_request =
            Self::build_estimate_fee_request(&request, new_txs, simulation_flags, block_id)?;

        let response = self.service.call(new_request).await;
        let response_body = response.as_json().get();
        let res = serde_json::from_str::<Response<'_, Vec<FeeEstimate>>>(response_body).map_err(
            |err| {
                Self::controller_deployment_error(format!(
                    "failed to parse estimateFee response: {err}"
                ))
            },
        )?;

        match res.payload {
            ResponsePayload::Success(estimates) => {
                if estimates.len() != new_txs_count {
                    return Err(Self::controller_deployment_error(format!(
                        "unexpected estimateFee response length: expected {new_txs_count}, got {}",
                        estimates.len()
                    )));
                }

                Ok(build_no_fee_response(&request, original_txs_count))
            }

            ResponsePayload::Error(..) => Ok(response),
        }
    }

    async fn cartridge_add_execute_from_outside_inner(
        &self,
        params: AddExecuteOutsideParams,
    ) -> Result<(), CartridgeApiError> {
        let address = params.address;
        let block_id = BlockIdOrTag::PreConfirmed;

        // check if the address has already been deployed.
        let is_deployed = match self.context.starknet.class_hash_at_address(block_id, address).await
        {
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

        let nonce = self
            .context
            .starknet
            .nonce_at(block_id, self.context.deployer_address)
            .await
            .map_err(|err| {
            Self::controller_deployment_error(format!("failed to get deployer nonce: {err}"))
        })?;
        let deploy_tx = self
            .get_controller_deployment_tx(address, nonce)
            .await
            .map_err(|err| Self::controller_deployment_error(err.to_string()))?;

        // None means the address is not of a Controller
        if let Some(tx) = deploy_tx {
            if let Err(e) = self.context.starknet.add_invoke_tx(tx).await {
                return Err(CartridgeApiError::ControllerDeployment {
                    reason: format!("failed to submit deployment tx: {e}"),
                });
            }
        }

        Ok(())
    }

    async fn get_controller_deployment_txs(
        &self,
        controller_addresses: Vec<ContractAddress>,
        initial_nonce: Nonce,
    ) -> Result<Vec<BroadcastedTx>, Error> {
        let mut deploy_transactions: Vec<BroadcastedTx> = Vec::new();
        let mut processed_addresses: HashSet<ContractAddress> = HashSet::new();

        let mut deployer_nonce = initial_nonce;

        for address in controller_addresses {
            // If the address has already been processed in this txs batch, just skip.
            if processed_addresses.contains(&address) {
                continue;
            }

            let deploy_tx = self.get_controller_deployment_tx(address, deployer_nonce).await?;

            // None means the address is not a Controller
            if let Some(tx) = deploy_tx {
                deployer_nonce += Nonce::ONE;
                processed_addresses.insert(address);
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
        let Some(ctor_calldata) = self.context.cartridge_api.get_account_calldata(address).await?
        else {
            // this means no controller with the given address
            return Ok(None);
        };

        let call = Call {
            contract_address: DEFAULT_UDC_ADDRESS,
            calldata: ctor_calldata.constructor_calldata,
            entry_point_selector: selector!("deployContract"),
        };

        let mut tx = BroadcastedInvokeTx {
            sender_address: self.context.deployer_address,
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
            let chain = self.context.starknet.chain_id();
            let tx = BroadcastedTx::Invoke(tx.clone());
            let tx = BroadcastedTxWithChainId { tx, chain: chain.into() };

            let signer = LocalWallet::from(self.context.deployer_private_key.clone());

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
    Pool: TransactionPool<Transaction = PoolTx> + 'static,
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
                STARKNET_ESTIMATE_FEE => {
                    trace!(%method, "Intercepting JSON-RPC method.");
                    if let Some(params) = parse_estimate_fee_params(&request) {
                        return this.starknet_estimate_fee(params, request).await;
                    }
                }

                CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE | CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE_TX => {
                    trace!(%method, "Intercepting JSON-RPC method.");
                    if let Some(params) = parse_execute_outside_params(&request) {
                        return this.cartridge_add_execute_from_outside(params, request).await;
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cartridge api error: {0}")]
    Client(#[from] cartridge::api::Error),

    #[error("failed to sign deploy transaction: {0}")]
    SigningError(SignError),
}

#[allow(dead_code)]
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

#[derive(Deserialize)]
struct AddExecuteOutsidePositionalParams(
    ContractAddress,
    OutsideExecution,
    Vec<Felt>,
    #[serde(default)] Option<FeeSource>,
);

#[derive(Deserialize)]
#[serde(untagged)]
enum AddExecuteOutsideRequestParams {
    Named(AddExecuteOutsideParams),
    Positional(AddExecuteOutsidePositionalParams),
}

impl From<AddExecuteOutsideRequestParams> for AddExecuteOutsideParams {
    fn from(value: AddExecuteOutsideRequestParams) -> Self {
        match value {
            AddExecuteOutsideRequestParams::Named(params) => params,
            AddExecuteOutsideRequestParams::Positional(params) => Self {
                address: params.0,
                outside_execution: params.1,
                signature: params.2,
                fee_source: params.3,
            },
        }
    }
}

#[derive(Deserialize)]
struct EstimateFeePositionalParams(
    Vec<BroadcastedTx>,
    Vec<SimulationFlagForEstimateFee>,
    BlockIdOrTag,
);

#[derive(Deserialize)]
#[serde(untagged)]
enum EstimateFeeRequestParams {
    Named(EstimateFeeParams),
    Positional(EstimateFeePositionalParams),
}

impl From<EstimateFeeRequestParams> for EstimateFeeParams {
    fn from(value: EstimateFeeRequestParams) -> Self {
        match value {
            EstimateFeeRequestParams::Named(params) => params,
            EstimateFeeRequestParams::Positional(params) => {
                Self { transactions: params.0, simulation_flags: params.1, block_id: params.2 }
            }
        }
    }
}

fn parse_params<T: DeserializeOwned>(request: &Request<'_>, method: &str) -> Option<T> {
    match request.params().parse() {
        Ok(params) => Some(params),
        Err(..) => {
            debug!(target: "cartridge", "Failed to parse {method} params.");
            None
        }
    }
}

fn parse_execute_outside_params(request: &Request<'_>) -> Option<AddExecuteOutsideParams> {
    parse_params::<AddExecuteOutsideRequestParams>(request, "execute outside").map(Into::into)
}

/// Extract estimate_fee parameters from the request.
fn parse_estimate_fee_params(request: &Request<'_>) -> Option<EstimateFeeParams> {
    parse_params::<EstimateFeeRequestParams>(request, "estimate fee").map(Into::into)
}

// Temporary shim for --dev.no-fee when deployment txs are prepended for controllers.
// Remove once starknet_estimateFee natively returns zeroed fees in this scenario.
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
