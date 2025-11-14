use std::borrow::Cow;
use std::future::Future;

use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::Request;
use jsonrpsee::MethodResponse;
use katana_executor::ExecutorFactory;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::broadcasted::BroadcastedTx;
use serde::Deserialize;
use starknet::core::types::SimulationFlagForEstimateFee;

use super::{Error, Paymaster};
use crate::rpc::types::OutsideExecution;

#[derive(Debug)]
pub struct PaymasterLayer<EF: ExecutorFactory> {
    pub(crate) paymaster: Paymaster<EF>,
}

impl<EF: ExecutorFactory> Clone for PaymasterLayer<EF> {
    fn clone(&self) -> Self {
        Self { paymaster: self.paymaster.clone() }
    }
}

impl<S, EF: ExecutorFactory> tower::Layer<S> for PaymasterLayer<EF> {
    type Service = PaymasterService<S, EF>;

    fn layer(&self, service: S) -> Self::Service {
        PaymasterService { service, paymaster: self.paymaster.clone() }
    }
}

#[derive(Debug)]
pub struct PaymasterService<S, EF: ExecutorFactory> {
    service: S,
    paymaster: Paymaster<EF>,
}

impl<S, EF> PaymasterService<S, EF>
where
    S: RpcServiceT + Send + Sync + Clone + 'static,
    EF: ExecutorFactory,
{
    async fn intercept_estimate_fee(paymaster: Paymaster<EF>, request: &mut Request<'_>) {
        let params = request.params();

        let (txs, simulation_flags, block_id) = if params.is_object() {
            #[derive(Deserialize)]
            struct ParamsObject {
                request: Vec<BroadcastedTx>,
                #[serde(alias = "simulationFlags")]
                simulation_flags: Vec<SimulationFlagForEstimateFee>,
                #[serde(alias = "blockId")]
                block_id: BlockIdOrTag,
            }

            let parsed: ParamsObject = match params.parse() {
                Ok(p) => p,
                Err(..) => return,
            };

            (parsed.request, parsed.simulation_flags, parsed.block_id)
        } else {
            let mut seq = params.sequence();

            let txs_result: Result<Vec<BroadcastedTx>, _> = seq.next();
            let simulation_flags_result: Result<Vec<SimulationFlagForEstimateFee>, _> = seq.next();
            let block_id_result: Result<BlockIdOrTag, _> = seq.next();

            match (txs_result, simulation_flags_result, block_id_result) {
                (Ok(txs), Ok(simulation_flags), Ok(block_id)) => (txs, simulation_flags, block_id),
                _ => return,
            }
        };

        if let Ok(new_txs) = paymaster.handle_estimate_fees(block_id, txs).await {
            let new_params = {
                let mut params = jsonrpsee::core::params::ArrayParams::new();
                params.insert(new_txs).unwrap();
                params.insert(simulation_flags).unwrap();
                params.insert(block_id).unwrap();
                params
            };

            let params = new_params.to_rpc_params().unwrap();
            let params = params.map(Cow::Owned);
            request.params = params;
        }
    }

    async fn intercept_add_outside_execution(paymaster: Paymaster<EF>, request: &mut Request<'_>) {
        let params = request.params();

        let (controller_address, outside_execution, signature) = if params.is_object() {
            #[derive(Deserialize)]
            struct ParamsObject {
                address: ContractAddress,
                #[serde(alias = "outsideExecution")]
                outside_execution: OutsideExecution,
                signature: Vec<Felt>,
            }

            let parsed: ParamsObject = match params.parse() {
                Ok(p) => p,
                Err(..) => return,
            };

            (parsed.address, parsed.outside_execution, parsed.signature)
        } else {
            let mut seq = params.sequence();

            let address_result: Result<ContractAddress, _> = seq.next();
            let outside_execution_result: Result<OutsideExecution, _> = seq.next();
            let signature_result: Result<Vec<Felt>, _> = seq.next();

            match (address_result, outside_execution_result, signature_result) {
                (Ok(address), Ok(outside_execution), Ok(signature)) => {
                    (address, outside_execution, signature)
                }
                _ => return,
            }
        };

        match paymaster
            .handle_add_outside_execution(controller_address, outside_execution, signature)
            .await
        {
            Ok(Some((outside_execution, signature))) => {
                let new_params = {
                    let mut params = jsonrpsee::core::params::ArrayParams::new();
                    params.insert(controller_address).unwrap();
                    params.insert(outside_execution).unwrap();
                    params.insert(signature).unwrap();
                    params
                };

                let params = new_params.to_rpc_params().unwrap();
                let params = params.map(Cow::Owned);
                request.params = params;
            }
            Ok(None) | Err(Error::ControllerNotFound(..)) => {}
            Err(error) => panic!("{error}"),
        }
    }
}

impl<S, EF> Clone for PaymasterService<S, EF>
where
    S: Clone,
    EF: ExecutorFactory + Clone,
{
    fn clone(&self) -> Self {
        Self { service: self.service.clone(), paymaster: self.paymaster.clone() }
    }
}

impl<S, EF> RpcServiceT for PaymasterService<S, EF>
where
    EF: ExecutorFactory,
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
        mut request: Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let service = self.service.clone();
        let paymaster = self.paymaster.clone();

        async move {
            if request.method_name() == "starknet_estimateFee" {
                Self::intercept_estimate_fee(paymaster.clone(), &mut request).await;
            } else if request.method_name() == "cartridge_addExecuteOutsideTransaction" {
                Self::intercept_add_outside_execution(paymaster, &mut request).await;
            }

            service.call(request).await
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
