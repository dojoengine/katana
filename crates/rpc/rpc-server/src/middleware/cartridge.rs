use std::borrow::Cow;
use std::collections::HashSet;
use std::future::Future;

use cartridge::vrf::VrfClientError;
use cartridge::CartridgeApiClient;
use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::http_client::HttpClient;
use jsonrpsee::types::Request;
use jsonrpsee::MethodResponse;
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_pool::api::{PoolError, TransactionPool};
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::execution::Call;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::ContractAddress;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_client::starknet::StarknetApiError;
use katana_rpc_types::broadcasted::{BroadcastedTx, BroadcastedTxWithChainId};
use katana_rpc_types::{BroadcastedInvokeTx, FeeEstimate};
use serde::Deserialize;
use starknet::core::types::SimulationFlagForEstimateFee;
use starknet::macros::selector;
use starknet::providers::jsonrpc::JsonRpcResponse;
use starknet::signers::local_wallet::SignError;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tracing::{debug, trace};

use crate::cartridge::{encode_calls, VrfService};
use crate::starknet::{PendingBlockProvider, StarknetApi};

#[derive(Debug, Clone)]
pub struct ControllerDeploymentLayer<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    inner: ControllerDeployment<Pool, PP, PF>,
}

impl<S, Pool, PP, PF> tower::Layer<S> for ControllerDeploymentLayer<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    type Service = ControllerDeploymentService<S, Pool, PP, PF>;

    fn layer(&self, service: S) -> Self::Service {
        ControllerDeploymentService { service, inner: self.inner.clone() }
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

#[derive(Debug, Clone)]
pub struct ControllerDeployment<Pool, PP, PF>
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
}

impl<Pool, PP, PF> ControllerDeployment<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    pub fn new(
        starknet: StarknetApi<Pool, PP, PF>,
        cartridge_api: CartridgeApiClient,
        paymaster_client: HttpClient,
        vrf_service: Option<VrfService>,
    ) -> Self {
        Self { starknet, cartridge_api, paymaster_client, vrf_service }
    }

    pub async fn handle_estimate_fee_inner(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
    ) -> Result<Vec<BroadcastedTx>, Error> {
        let mut new_transactions: Vec<BroadcastedTx> = Vec::new();
        let mut updated_transactions: Vec<BroadcastedTx> = Vec::new();
        let mut deployed_controllers: HashSet<ContractAddress> = HashSet::new();

        let mut deployer_nonce =
            self.starknet.nonce_at(block_id, self.deployer_address).await.unwrap();

        // iterate thru all txs and deploy any undeployed contract (if they are a Controller)
        for tx in transactions {
            let contract_address = match &tx {
                BroadcastedTx::Invoke(tx) => tx.sender_address,
                BroadcastedTx::Declare(tx) => tx.sender_address,
                _ => continue,
            };

            // If the address has already been processed in this txs batch, just skip.
            if deployed_controllers.contains(&contract_address) {
                continue;
            }

            // check if the address has already been deployed.
            match self.starknet.class_hash_at_address(block_id, contract_address).await {
                // attempt to deploy if the address belongs to a Controller account
                Err(StarknetApiError::ContractNotFound) => {
                    let result = self
                        .get_controller_deployment_tx(contract_address, deployer_nonce)
                        .await?
                        .map(BroadcastedTx::Invoke);

                    // none means the address is not a Controller
                    if let Some(tx) = result {
                        deployed_controllers.insert(contract_address);
                        new_transactions.push(tx);
                        deployer_nonce += Nonce::ONE;
                    }
                }

                Err(e) => panic!("{}", e.to_string()),
                Ok(..) => continue,
            }
        }

        if new_transactions.is_empty() {
            Ok(transactions)
        } else {
            new_transactions.extend(updated_transactions);
            Ok(new_transactions)
        }
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

#[derive(Debug, Clone)]
pub struct ControllerDeploymentService<S, Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    inner: ControllerDeployment<Pool, PP, PF>,
    service: S,
}

impl<S, Pool, PP, PF> ControllerDeploymentService<S, Pool, PP, PF>
where
    S: RpcServiceT<MethodResponse = MethodResponse>,
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn handle_estimate_fee<'a>(&self, request: Request<'a>) -> S::MethodResponse {
        let Some(params) = parse_estimate_fee_params(&request) else {
            return self.service.call(request).await;
        };

        let updated_txs = self
            .inner
            .handle_estimate_fee_inner(params.block_id, params.txs)
            .await
            .unwrap_or_default();

        // if `handle_estimate_fees` has added some new transactions at the
        // beginning of updated_txs, we have to remove
        // extras results from estimate_fees to be
        // sure to return the same number of result than the number
        // of transactions in the request.
        let nb_of_txs = params.txs.len();
        let nb_of_extra_txs = updated_txs.len() - nb_of_txs;

        let new_request = build_new_estimate_fee_request(&request, &params, updated_txs);
        let response = self.service.call(new_request).await;

        if response.is_success() && nb_of_extra_txs > 0 {
            if let Ok(JsonRpcResponse::Success { result: mut estimate_fees, .. }) =
                serde_json::from_str::<JsonRpcResponse<Vec<FeeEstimate>>>(response.to_json().get())
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

        // TODO: restore the real response
        build_no_fee_response(&request, nb_of_txs)
    }
}

impl<S, Pool, PP, PF> RpcServiceT for ControllerDeploymentService<S, Pool, PP, PF>
where
    S: RpcServiceT + Send + Sync + Clone + 'static,
    S: RpcServiceT<
        MethodResponse = MethodResponse,
        BatchResponse = MethodResponse,
        NotificationResponse = MethodResponse,
    >,
    Pool: TransactionPool + 'static,
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
        async move {
            if request.method_name() == "starknet_estimateFee" {
                self.handle_estimate_fee(request).await
            } else {
                self.service.call(request).await
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

#[derive(Deserialize)]
struct EstimateFeeParams {
    #[serde(alias = "request")]
    txs: Vec<BroadcastedTx>,
    #[serde(alias = "simulationFlags")]
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    #[serde(alias = "blockId")]
    block_id: BlockIdOrTag,
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
    updated_txs: Vec<BroadcastedTx>,
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
