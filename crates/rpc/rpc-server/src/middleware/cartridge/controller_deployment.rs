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
use katana_primitives::ContractAddress;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::error::cartridge::CartridgeApiError;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::broadcasted::{BroadcastedTx, BroadcastedTxWithChainId};
use katana_rpc_types::{BroadcastedInvokeTx, FeeEstimate};
use starknet::core::types::SimulationFlagForEstimateFee;
use starknet::macros::selector;
use starknet::signers::local_wallet::SignError;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tower::Layer;
use tracing::{debug, trace};

use super::utils::{
    parse_params, AddExecuteOutsideParams, AddExecuteOutsideRequestParams, EstimateFeeParams,
    EstimateFeeRequestParams, CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE,
    CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE_TX, STARKNET_ESTIMATE_FEE,
};
use crate::cartridge::encode_calls;
use crate::starknet::{PendingBlockProvider, StarknetApi};

#[derive(Debug)]
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

impl<Pool, PP, PF> Clone for ControllerDeploymentContext<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self {
            starknet: self.starknet.clone(),
            cartridge_api: self.cartridge_api.clone(),
            deployer_address: self.deployer_address,
            deployer_private_key: self.deployer_private_key.clone(),
        }
    }
}

#[derive(Debug)]
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

#[derive(Debug)]
pub struct ControllerDeploymentService<S, Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
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
    async fn estimate_fee_candidate_addresses(
        &self,
        block_id: BlockIdOrTag,
        transactions: &[BroadcastedTx],
    ) -> Result<Vec<ContractAddress>, CartridgeApiError> {
        let mut undeployed_address = Vec::with_capacity(transactions.len());

        for tx in transactions {
            let address = match tx {
                BroadcastedTx::Invoke(tx) => tx.sender_address,
                BroadcastedTx::Declare(tx) => tx.sender_address,
                _ => continue,
            };

            match self.context.starknet.class_hash_at_address(block_id, address).await {
                Err(StarknetApiError::ContractNotFound) => undeployed_address.push(address),

                Ok(..) => {}
                Err(e) => {
                    return Err(CartridgeApiError::ControllerDeployment { error: Box::new(e) })
                }
            };
        }

        Ok(undeployed_address)
    }

    fn build_estimate_fee_request<'a>(
        request: &Request<'a>,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockIdOrTag,
    ) -> Result<Request<'a>, CartridgeApiError> {
        let params = rpc_params!(transactions, simulation_flags, block_id);
        let params = params
            .to_rpc_params()
            .map_err(|err| CartridgeApiError::ControllerDeployment { error: Box::new(err) })?;

        let mut new_request = request.clone();
        new_request.params = params.map(Cow::Owned);

        Ok(new_request)
    }

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
        let candidate_addresses =
            self.estimate_fee_candidate_addresses(block_id, &transactions).await?;

        let deployer_nonce = self
            .context
            .starknet
            .nonce_at(block_id, self.context.deployer_address)
            .await
            .map_err(|err| CartridgeApiError::ControllerDeployment { error: Box::new(err) })?;

        let deploy_controller_txs = self
            .get_controller_deployment_txs(candidate_addresses, deployer_nonce)
            .await
            .map_err(|err| CartridgeApiError::ControllerDeployment { error: Box::new(err) })?;

        // No Controller to deploy, simply forward the request.
        if deploy_controller_txs.is_empty() {
            trace!(target: "middleware::cartridge", "No controller to deploy - forwarding request.");
            return Ok(self.service.call(request).await);
        }

        let og_txs_len = transactions.len();
        let deploy_txs_len = deploy_controller_txs.len();
        trace!(target: "middleware::cartridge", deployment_count = deploy_txs_len, "Prepending controller deployment transactions to estimate fee.");

        // Build a new estimateFee request with the deploy controller transactions prepended.
        let new_estimates_txs = [deploy_controller_txs, transactions].concat();
        let updated_request = Self::build_estimate_fee_request(
            &request,
            new_estimates_txs,
            simulation_flags,
            block_id,
        )?;

        // Call the inner service with the updated request.
        let response = self.service.call(updated_request).await;

        // IMPORTANT:
        //
        // Beyond this point, we assume the response is a valid starknet_estimateFee response. It's
        // up to the devs to make sure this middleware is wrapped around valid Starknet
        // JSON-RPC service and no other middlewares are installed that modify the response
        // body.

        // Extract the response body to remove the estimates for the deploy controller transactions
        // before returning the response to the caller.
        //
        // This is done to respect the semantics of the starknet_estimateFee response, which
        // specifies that the estimates are only for transactions that are included in the
        // original request.
        let response_body = response.as_json().get();
        let res = serde_json::from_str::<Response<'_, Vec<FeeEstimate>>>(response_body)
            .map_err(|err| CartridgeApiError::ControllerDeployment { error: Box::new(err) })?;

        match res.payload {
            ResponsePayload::Success(estimates) => {
                let estimates_len = estimates.len();
                let new_txs_len = deploy_txs_len + og_txs_len;
                assert_eq!(
                    estimates_len, new_txs_len,
                    "unexpected estimateFee response length: expected {new_txs_len}, got \
                     {estimates_len}"
                );

                let og_estimates = estimates[deploy_txs_len..].to_vec();
                let payload = ResponsePayload::success(og_estimates);

                Ok(MethodResponse::response(request.id().clone(), payload.into(), usize::MAX))
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

            Err(e) => return Err(CartridgeApiError::ControllerDeployment { error: Box::new(e) }),
        };

        if is_deployed {
            trace!(target: "middleware::cartridge", %address, "Controller account contract already deployed.");
            return Ok(());
        }

        let nonce =
            self.context
                .starknet
                .nonce_at(block_id, self.context.deployer_address)
                .await
                .map_err(|err| CartridgeApiError::ControllerDeployment { error: Box::new(err) })?;

        let deploy_tx = self
            .get_controller_deployment_tx(address, nonce)
            .await
            .map_err(|err| CartridgeApiError::ControllerDeployment { error: Box::new(err) })?;

        // None means the address is not of a Controller in which case we don't need to do anything.
        if let Some(tx) = deploy_tx {
            trace!(target: "middleware::cartridge", %address, "Deploying Controller account contract.");

            // Add the deploy tx to the pool.
            //
            // Wait for the transaction to be executed before proceeding.
            if let Err(e) = self.context.starknet.add_invoke_tx_sync(tx).await {
                return Err(CartridgeApiError::ControllerDeployment { error: Box::new(e) });
            }
        } else {
            trace!(target: "middleware::cartridge", %address, "Account contract is not a Controller.");
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

    /// Returns the deployment transaction for the given address, if it is a Controller. None means
    /// the address is not a Controller.
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
        let this = (*self).clone();

        async move {
            let method = request.method_name();

            match method {
                STARKNET_ESTIMATE_FEE => {
                    trace!(target: "middleware::cartridge::controller", %method, "Intercepting JSON-RPC method.");
                    if let Ok(params) = parse_params::<EstimateFeeRequestParams>(&request)
                        .inspect_err(|error| debug!(target: "middleware::cartridge", %method, %error, "Failed to parse params."))
                    {
                        return this.starknet_estimate_fee(params.into(), request).await;
                    }
                }

                CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE | CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE_TX => {
                    trace!(target: "middleware::cartridge::controller", %method, "Intercepting JSON-RPC method.");
                    if let Ok(params) = parse_params::<AddExecuteOutsideRequestParams>(&request)
	                    .inspect_err(|error| debug!(target: "middleware::cartridge", %method, %error, "Failed to parse params."))
                    {
                        return this.cartridge_add_execute_from_outside(params.into(), request).await;
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

impl<Pool, PP, PF> Clone for ControllerDeploymentLayer<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self { context: self.context.clone() }
    }
}

impl<S, Pool, PP, PF> Clone for ControllerDeploymentService<S, Pool, PP, PF>
where
    S: Clone,
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self { context: self.context.clone(), service: self.service.clone() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cartridge api error: {0}")]
    Client(#[from] cartridge::api::Error),

    #[error("failed to sign deploy transaction: {0}")]
    SigningError(SignError),
}
