use std::borrow::Cow;
use std::future::Future;

use jsonrpsee::core::middleware;
use jsonrpsee::core::middleware::{Batch, Notification};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::Request;
use katana_executor::ExecutorFactory;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::transaction::BroadcastedTx;
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

impl<S, EF> Clone for PaymasterService<S, EF>
where
    S: Clone,
    EF: ExecutorFactory + Clone,
{
    fn clone(&self) -> Self {
        Self { service: self.service.clone(), paymaster: self.paymaster.clone() }
    }
}

impl<S, EF> PaymasterService<S, EF>
where
    S: middleware::RpcServiceT + Send + Sync + Clone + 'static,
    EF: ExecutorFactory,
{
    fn call_on_estimate_fee(&self, request: &mut Request<'_>) {
        let params = request.params();

        let (mut requests, simulation_flags, block_id) = if params.is_object() {
            #[derive(serde::Deserialize)]
            struct ParamsObject<G0, G1, G2> {
                request: G0,
                #[serde(alias = "simulationFlags")]
                simulation_flags: G1,
                #[serde(alias = "blockId")]
                block_id: G2,
            }

            let parsed: ParamsObject<
                Vec<BroadcastedTx>,
                Vec<SimulationFlagForEstimateFee>,
                BlockIdOrTag,
            > = match params.parse() {
                Ok(p) => p,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse_as_object(&e);
                    // return jsonrpsee::ResponsePayload::error(e);
                    todo!()
                }
            };
            (parsed.request, parsed.simulation_flags, parsed.block_id)
        } else {
            let mut seq = params.sequence();
            let request: Vec<BroadcastedTx> = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "request",
                        "Vec < BroadcastedTx >",
                        &e,
                        false,
                    );
                    // return jsonrpsee::ResponsePayload::error(e);
                    todo!()
                }
            };
            let simulation_flags: Vec<SimulationFlagForEstimateFee> = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "simulation_flags",
                        "Vec < SimulationFlagForEstimateFee >",
                        &e,
                        false,
                    );
                    // return jsonrpsee::ResponsePayload::error(e);
                    todo!()
                }
            };
            let block_id: BlockIdOrTag = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "block_id",
                        "BlockIdOrTag",
                        &e,
                        false,
                    );
                    // return jsonrpsee::ResponsePayload::error(e);
                    todo!()
                }
            };
            (request, simulation_flags, block_id)
        };

        let new_params = {
            let mut params = jsonrpsee::core::params::ArrayParams::new();

            if let Err(err) = params.insert(requests) {
                jsonrpsee::core::__reexports::panic_fail_serialize("request", err);
            }
            if let Err(err) = params.insert(simulation_flags) {
                jsonrpsee::core::__reexports::panic_fail_serialize("simulation_flags", err);
            }
            if let Err(err) = params.insert(block_id) {
                jsonrpsee::core::__reexports::panic_fail_serialize("block_id", err);
            }

            params
        };

        let params = new_params.to_rpc_params().unwrap();
        let params = params.map(Cow::Owned);
        request.params = params;
    }

    fn call_on_add_outside_execution(&self, request: &Request<'_>) {
        let params = request.params();

        let (controller_address, ..) = if params.is_object() {
            #[derive(serde::Deserialize)]
            struct ParamsObject<G0, G1, G2> {
                address: G0,
                #[serde(alias = "outsideExecution")]
                outside_execution: G1,
                signature: G2,
            }
            let parsed: ParamsObject<ContractAddress, OutsideExecution, Vec<Felt>> =
                match params.parse() {
                    Ok(p) => p,
                    Err(e) => {
                        jsonrpsee::core::__reexports::log_fail_parse_as_object(&e);
                        return;
                    }
                };
            (parsed.address, parsed.outside_execution, parsed.signature)
        } else {
            let mut seq = params.sequence();
            let address: ContractAddress = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "address",
                        "ContractAddress",
                        &e,
                        false,
                    );
                    return;
                }
            };
            let outside_execution: OutsideExecution = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "outside_execution",
                        "OutsideExecution",
                        &e,
                        false,
                    );
                    return;
                }
            };
            let signature: Vec<Felt> = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "signature",
                        "Vec < Felt >",
                        &e,
                        false,
                    );
                    return;
                }
            };
            (address, outside_execution, signature)
        };

        match self.paymaster.deploy_controller(controller_address) {
            Ok(tx_hash) => {
                trace!(
                    tx_hash = format!("{tx_hash:#x}"),
                    "Controller deploy transaction submitted",
                );
            }
            Err(Error::ControllerNotFound(..)) => {}
            Err(error) => panic!("{error}"),
        }
    }
}

impl<S, EF> middleware::RpcServiceT for PaymasterService<S, EF>
where
    S: middleware::RpcServiceT + Send + Sync + Clone + 'static,
    EF: ExecutorFactory,
{
    type BatchResponse = S::BatchResponse;
    type MethodResponse = S::MethodResponse;
    type NotificationResponse = S::NotificationResponse;

    fn call<'a>(
        &self,
        mut request: Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        if request.method_name() == "starknet_estimateFee" {
            self.call_on_estimate_fee(&mut request);
            self.service.call(request)
        } else if request.method_name() == "cartridge_addExecuteOutsideTransaction" {
            self.call_on_add_outside_execution(&request);
            self.service.call(request)
        } else {
            self.service.call(request)
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
