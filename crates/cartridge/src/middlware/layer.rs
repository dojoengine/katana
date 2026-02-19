use std::borrow::Cow;
use std::future::Future;

use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::Request;
use jsonrpsee::MethodResponse;
use katana_primitives::block::BlockIdOrTag;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::FeeEstimate;
use serde::Deserialize;
use starknet::core::types::SimulationFlagForEstimateFee;
use starknet::providers::jsonrpc::JsonRpcResponse;
use tracing::{debug, trace};

use super::ControllerDeployment;

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
        paymaster: ControllerDeployment,
        request: Request<'a>,
    ) -> S::MethodResponse {
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
