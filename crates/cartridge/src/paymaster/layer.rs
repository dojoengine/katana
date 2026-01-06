use std::borrow::Cow;
use std::future::Future;

use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::Request;
use jsonrpsee::MethodResponse;
use katana_core::backend::storage::ProviderRO;
use katana_pool::TransactionPool;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::ProviderFactory;
use katana_rpc_server::starknet::PendingBlockProvider;
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::FeeEstimate;
use serde::Deserialize;
use starknet::core::types::SimulationFlagForEstimateFee;
use starknet::providers::jsonrpc::JsonRpcResponse;
use tracing::{debug, trace};

use super::Paymaster;
use crate::rpc::types::OutsideExecution;

#[derive(Deserialize)]
struct EstimateFeeParams {
    #[serde(alias = "request")]
    txs: Vec<BroadcastedTx>,
    #[serde(alias = "simulationFlags")]
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    #[serde(alias = "blockId")]
    block_id: BlockIdOrTag,
}

#[derive(Deserialize)]
struct OutsideExecutionParams {
    #[serde(alias = "address")]
    controller_address: ContractAddress,
    #[serde(alias = "outsideExecution")]
    outside_execution: OutsideExecution,
    signature: Vec<Felt>,
}

#[derive(Debug)]
pub struct PaymasterLayer<Pool: TransactionPool, PP: PendingBlockProvider, PF: ProviderFactory>
where
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    pub(crate) paymaster: Paymaster<Pool, PP, PF>,
}

impl<Pool: TransactionPool, PP: PendingBlockProvider, PF: ProviderFactory> Clone
    for PaymasterLayer<Pool, PP, PF>
where
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    fn clone(&self) -> Self {
        Self { paymaster: self.paymaster.clone() }
    }
}

impl<S, Pool: TransactionPool, PP: PendingBlockProvider, PF: ProviderFactory> tower::Layer<S>
    for PaymasterLayer<Pool, PP, PF>
where
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    type Service = PaymasterService<S, Pool, PP, PF>;

    fn layer(&self, service: S) -> Self::Service {
        PaymasterService { service, paymaster: self.paymaster.clone() }
    }
}

#[derive(Debug)]
pub struct PaymasterService<S, Pool: TransactionPool, PP: PendingBlockProvider, PF: ProviderFactory>
where
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    service: S,
    paymaster: Paymaster<Pool, PP, PF>,
}

impl<S, Pool: TransactionPool + 'static, PP: PendingBlockProvider, PF: ProviderFactory>
    PaymasterService<S, Pool, PP, PF>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send + Sync + Clone + 'static,
    <PF as ProviderFactory>::Provider: ProviderRO,
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

    /// Extract add_outside_execution parameters from the request.
    fn parse_add_outside_execution_params(request: &Request<'_>) -> Option<OutsideExecutionParams> {
        let params = request.params();

        if params.is_object() {
            match params.parse() {
                Ok(p) => Some(p),
                Err(..) => {
                    debug!(target: "cartridge", "Failed to parse outside execution params.");
                    None
                }
            }
        } else {
            let mut seq = params.sequence();

            let address_result: Result<ContractAddress, _> = seq.next();
            let outside_execution_result: Result<OutsideExecution, _> = seq.next();
            let signature_result: Result<Vec<Felt>, _> = seq.next();

            match (address_result, outside_execution_result, signature_result) {
                (Ok(controller_address), Ok(outside_execution), Ok(signature)) => {
                    Some(OutsideExecutionParams {
                        controller_address,
                        outside_execution,
                        signature,
                    })
                }
                _ => {
                    debug!(target: "cartridge", "Failed to parse outside execution params.");
                    None
                }
            }
        }
    }

    fn build_new_outside_execution_request<'a>(
        request: &Request<'a>,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
    ) -> Request<'a> {
        let mut new_request = request.clone();

        let mut new_params = jsonrpsee::core::params::ArrayParams::new();
        new_params.insert(address).unwrap();
        new_params.insert(outside_execution).unwrap();
        new_params.insert(signature).unwrap();

        let new_params = new_params.to_rpc_params().unwrap();
        new_request.params = new_params.map(Cow::Owned);
        new_request
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

    /// <----
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
    /// ---->

    fn intercept_estimate_fee<'a>(
        service: S,
        paymaster: Paymaster<Pool, PP, PF>,
        request: Request<'a>,
    ) -> impl Future<Output = S::MethodResponse> + Send + 'a {
        async move {
            if let Some(params) = Self::parse_estimate_fee_params(&request) {
                let updated_txs = paymaster
                    .handle_estimate_fees(params.block_id, &params.txs)
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
                            // return MethodResponse::response(
                            // request.id().clone(),
                            // jsonrpsee::ResponsePayload::success(estimate_fees),
                            // usize::MAX,
                            // );
                        }
                    }

                    trace!(target: "cartridge", "Estimate fee endpoint original response returned");
                    return Self::build_no_fee_response(&request, nb_of_txs);
                    //                        return response;
                }
            }

            trace!(target: "cartridge", "Estimate fee endpoint called with the original transaction");
            service.call(request).await
        }
    }

    fn intercept_add_outside_execution<'a>(
        service: S,
        paymaster: Paymaster<Pool, PP, PF>,
        request: Request<'a>,
    ) -> impl Future<Output = S::MethodResponse> + Send + 'a {
        async move {
            if let Some(OutsideExecutionParams {
                controller_address,
                outside_execution,
                signature,
            }) = Self::parse_add_outside_execution_params(&request)
            {
                let updated_tx = match paymaster
                    .handle_add_outside_execution(controller_address, outside_execution, signature)
                    .await
                {
                    Ok(Some(tx)) => Some(tx),
                    Ok(None) => None,
                    Err(error) => panic!("{error}"),
                };

                if let Some((outside_execution, signature)) = updated_tx {
                    let new_request = Self::build_new_outside_execution_request(
                        &request,
                        controller_address,
                        outside_execution,
                        signature,
                    );

                    trace!(target: "cartridge", "Call outside_execution endpoint with the updated transaction");
                    return service.call(new_request).await;
                }
            }

            trace!(target: "cartridge", "Call outside_execution endpoint with the original transaction");
            service.call(request).await
        }
    }
}

impl<S, Pool: TransactionPool, PP: PendingBlockProvider, PF: ProviderFactory> Clone
    for PaymasterService<S, Pool, PP, PF>
where
    S: Clone,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    fn clone(&self) -> Self {
        Self { service: self.service.clone(), paymaster: self.paymaster.clone() }
    }
}

impl<S, Pool: TransactionPool + 'static, PP: PendingBlockProvider, PF: ProviderFactory> RpcServiceT
    for PaymasterService<S, Pool, PP, PF>
where
    S: RpcServiceT<
            MethodResponse = MethodResponse,
            BatchResponse = MethodResponse,
            NotificationResponse = MethodResponse,
        > + Send
        + Sync
        + Clone
        + 'static,
    <PF as ProviderFactory>::Provider: ProviderRO,
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
                Self::intercept_estimate_fee(service, paymaster, request).await
            } else if request.method_name() == "cartridge_addExecuteOutsideTransaction" {
                Self::intercept_add_outside_execution(service, paymaster, request).await
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
