use std::borrow::Cow;
use std::future::Future;

use jsonrpsee::core::middleware::{self, Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::Request;
use jsonrpsee::MethodResponse;
use katana_executor::ExecutorFactory;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::broadcasted::BroadcastedTx;
use serde::Deserialize;
use starknet::core::types::SimulationFlagForEstimateFee;
use tracing::trace;

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
    S: middleware::RpcServiceT + Send + Sync + Clone + 'static,
    EF: ExecutorFactory,
{
    fn intercept_estimate_fee(&self, request: &mut Request<'_>) {
        let params = request.params();

        let (txs, simulation_flags, block_id) = if params.is_object() {
            #[derive(serde::Deserialize)]
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

            let request: Vec<BroadcastedTx> = match seq.next() {
                Ok(v) => v,
                Err(..) => return,
            };

            let simulation_flags: Vec<SimulationFlagForEstimateFee> = match seq.next() {
                Ok(v) => v,
                Err(..) => return,
            };

            let block_id: BlockIdOrTag = match seq.next() {
                Ok(v) => v,
                Err(..) => return,
            };

            (request, simulation_flags, block_id)
        };

        let new_txs = self.paymaster.handle_estimate_fees(block_id, txs).unwrap();

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

    fn intercept_add_outside_execution(&self, request: &Request<'_>) -> Option<MethodResponse> {
        let params = request.params();

        let (controller_address, ..) = if params.is_object() {
            #[derive(Deserialize)]
            struct ParamsObject {
                address: ContractAddress,
                #[serde(alias = "outsideExecution")]
                outside_execution: OutsideExecution,
                signature: Vec<Felt>,
            }

            let parsed: ParamsObject = match params.parse() {
                Ok(p) => p,
                Err(..) => return None,
            };

            (parsed.address, parsed.outside_execution, parsed.signature)
        } else {
            let mut seq = params.sequence();

            let address = match seq.next::<ContractAddress>() {
                Ok(v) => v,
                Err(..) => return None,
            };

            let outside_execution = match seq.next::<OutsideExecution>() {
                Ok(v) => v,
                Err(..) => return None,
            };

            let signature = match seq.next::<Vec<Felt>>() {
                Ok(v) => v,
                Err(..) => return None,
            };

            (address, outside_execution, signature)
        };

        match self.paymaster.deploy_controller(controller_address) {
            Ok(tx_hash) => {
                trace!(
                    target: "paymaster",
                    tx_hash = format!("{tx_hash:#x}"),
                    "Controller deploy transaction submitted",
                );

                None
            }
            Err(Error::ControllerNotFound(..)) => None,
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
        if request.method_name() == "starknet_estimateFee" {
            self.intercept_estimate_fee(&mut request);
        } else if request.method_name() == "cartridge_addExecuteOutsideTransaction" {
            self.intercept_add_outside_execution(&request);
        }

        self.service.call(request)
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
