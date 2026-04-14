use std::borrow::Cow;
use std::future::Future;

use cainome::cairo_serde::CairoSerde;
use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::{ErrorObjectOwned, Request};
use jsonrpsee::{rpc_params, MethodResponse};
use katana_primitives::chain::ChainId;
use katana_primitives::execution::Call;
use katana_primitives::Felt;
use katana_rpc_api::error::cartridge::CartridgeApiError;
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::outside_execution::OutsideExecution;
use katana_rpc_types::SignedOutsideExecution;
use starknet::macros::selector;
use tower::Layer;
use tracing::{debug, trace, trace_span, Instrument};

use super::utils::{
    parse_params, AddExecuteOutsideParams, AddExecuteOutsideRequestParams, EstimateFeeParams,
    EstimateFeeRequestParams, CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE,
    CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE_TX, STARKNET_ESTIMATE_FEE,
};
use crate::cartridge::{encode_calls, VrfService};

#[derive(Debug, Clone)]
struct VrfContext {
    vrf: VrfService,
    chain_id: ChainId,
}

#[derive(Debug, Clone)]
pub struct VrfLayer {
    context: VrfContext,
}

impl VrfLayer {
    pub fn new(vrf: VrfService, chain_id: ChainId) -> Self {
        Self { context: VrfContext { vrf, chain_id } }
    }
}

impl<S> Layer<S> for VrfLayer {
    type Service = VrfMiddlewareService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        VrfMiddlewareService { context: self.context.clone(), service: inner }
    }
}

#[derive(Debug, Clone)]
pub struct VrfMiddlewareService<S> {
    context: VrfContext,
    service: S,
}

impl<S> VrfMiddlewareService<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send + Sync + Clone + 'static,
{
    async fn cartridge_add_execute_from_outside<'a>(
        &self,
        params: AddExecuteOutsideParams,
        request: Request<'a>,
    ) -> MethodResponse {
        let request_id = request.id().clone();
        match self.cartridge_add_execute_from_outside_inner(params, &request).await {
            Ok(Some(new_request)) => self.service.call(new_request).await,
            Ok(None) => self.service.call(request).await,
            Err(err) => MethodResponse::error(request_id, ErrorObjectOwned::from(err)),
        }
    }

    async fn cartridge_add_execute_from_outside_inner<'a>(
        &self,
        params: AddExecuteOutsideParams,
        request: &Request<'a>,
    ) -> Result<Option<Request<'a>>, CartridgeApiError> {
        let AddExecuteOutsideParams { address, outside_execution, signature, fee_source } = params;

        let Some((req_rand_call, pos)) = find_and_get_request_random_call(&outside_execution)
        else {
            return Ok(None);
        };

        debug!(target: "middleware::cartridge::vrf", "Found a request_random call.");

        if (pos + 1) == outside_execution.calls().len() {
            return Err(CartridgeApiError::VrfMissingFollowUpCall);
        }

        if req_rand_call.contract_address != self.context.vrf.account_address() {
            return Err(CartridgeApiError::VrfInvalidTarget {
                requested: req_rand_call.contract_address,
                supported: self.context.vrf.account_address(),
            });
        }

        let vrf_signed_execution = self
            .context
            .vrf
            .outside_execution(
                &SignedOutsideExecution { address, outside_execution, signature },
                self.context.chain_id,
            )
            .instrument(trace_span!(target: "middleware::cartridge::vrf", "vrf.outside_execution"))
            .await?;

        let params = rpc_params!(
            vrf_signed_execution.address,
            vrf_signed_execution.outside_execution,
            vrf_signed_execution.signature,
            fee_source
        )
        .to_rpc_params()
        .map_err(|err| CartridgeApiError::VrfExecutionFailed {
            reason: format!("failed to build rpc params: {err}"),
        })?;

        let mut new_request = request.clone();
        new_request.params = params.map(Cow::Owned);
        Ok(Some(new_request))
    }

    async fn starknet_estimate_fee<'a>(
        &self,
        params: EstimateFeeParams,
        request: Request<'a>,
    ) -> MethodResponse {
        let request_id = request.id().clone();
        match self.starknet_estimate_fee_inner(params, &request).await {
            Ok(Some(new_request)) => self.service.call(new_request).await,
            Ok(None) => self.service.call(request).await,
            Err(err) => MethodResponse::error(request_id, ErrorObjectOwned::from(err)),
        }
    }

    async fn starknet_estimate_fee_inner<'a>(
        &self,
        params: EstimateFeeParams,
        request: &Request<'a>,
    ) -> Result<Option<Request<'a>>, CartridgeApiError> {
        let EstimateFeeParams { mut transactions, simulation_flags, block_id } = params;
        let mut rewritten = false;

        for tx in transactions.iter_mut() {
            let BroadcastedTx::Invoke(invoke) = tx else { continue };

            let current = invoke.calldata.clone();
            let new_calldata = match self.maybe_resolve_invoke_calldata(&current).await? {
                Some(data) => data,
                None => continue,
            };

            invoke.calldata = new_calldata;
            rewritten = true;
        }

        if !rewritten {
            return Ok(None);
        }

        let params = rpc_params!(transactions, simulation_flags, block_id);
        let params =
            params.to_rpc_params().map_err(|err| CartridgeApiError::VrfExecutionFailed {
                reason: format!("failed to build rpc params: {err}"),
            })?;

        let mut new_request = request.clone();
        new_request.params = params.map(Cow::Owned);
        Ok(Some(new_request))
    }

    /// Decodes an invoke tx calldata (cairo-1 multicall format) and, if it wraps an
    /// `execute_from_outside_v2/v3` call whose inner `OutsideExecution` contains a
    /// `request_random`, resolves the VRF and returns a re-encoded calldata.
    ///
    /// Returns `Ok(None)` if the calldata did not contain a VRF-bearing outside execution
    /// (including when decoding fails — VRF is opt-in and must not break unrelated txs).
    async fn maybe_resolve_invoke_calldata(
        &self,
        calldata: &[Felt],
    ) -> Result<Option<Vec<Felt>>, CartridgeApiError> {
        let mut calls = match Vec::<Call>::cairo_deserialize(calldata, 0) {
            Ok(calls) => calls,
            Err(err) => {
                trace!(target: "middleware::cartridge::vrf", %err, "Failed to decode invoke calldata as Vec<Call>.");
                return Ok(None);
            }
        };

        let mut rewritten = false;
        for call in calls.iter_mut() {
            if call.entry_point_selector != selector!("execute_from_outside_v2")
                && call.entry_point_selector != selector!("execute_from_outside_v3")
            {
                continue;
            }

            let outside_execution = match OutsideExecution::cairo_deserialize(&call.calldata, 0) {
                Ok(oe) => oe,
                Err(err) => {
                    trace!(target: "middleware::cartridge::vrf", %err, "Failed to decode OutsideExecution from inner call calldata.");
                    continue;
                }
            };

            let Some((req_rand_call, pos)) = find_and_get_request_random_call(&outside_execution)
            else {
                continue;
            };

            debug!(target: "middleware::cartridge::vrf", "Found a request_random call in estimateFee.");

            if (pos + 1) == outside_execution.calls().len() {
                return Err(CartridgeApiError::VrfMissingFollowUpCall);
            }

            if req_rand_call.contract_address != self.context.vrf.account_address() {
                return Err(CartridgeApiError::VrfInvalidTarget {
                    requested: req_rand_call.contract_address,
                    supported: self.context.vrf.account_address(),
                });
            }

            let oe_size = OutsideExecution::cairo_serialized_size(&outside_execution);
            let signature = match Vec::<Felt>::cairo_deserialize(&call.calldata, oe_size) {
                Ok(sig) => sig,
                Err(err) => {
                    trace!(target: "middleware::cartridge::vrf", %err, "Failed to decode signature tail of execute_from_outside call.");
                    continue;
                }
            };

            let signed = SignedOutsideExecution {
                address: call.contract_address,
                outside_execution,
                signature,
            };

            let resolved = self
                .context
                .vrf
                .outside_execution(&signed, self.context.chain_id)
                .instrument(trace_span!(
                    target: "middleware::cartridge::vrf",
                    "vrf.outside_execution"
                ))
                .await?;

            *call = Call::from(resolved);
            rewritten = true;
        }

        if rewritten {
            Ok(Some(encode_calls(calls)))
        } else {
            Ok(None)
        }
    }
}

impl<S> RpcServiceT for VrfMiddlewareService<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send + Sync + Clone + 'static,
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
                    trace!(target: "middleware::cartridge::vrf", %method, "Intercepting JSON-RPC method.");
                    if let Ok(params) = parse_params::<EstimateFeeRequestParams>(&request)
                        .inspect_err(|error| debug!(target: "middleware::cartridge::vrf", %method, %error, "Failed to parse params."))
                    {
                        return this.starknet_estimate_fee(params.into(), request).await;
                    }
                }

                CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE | CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE_TX => {
                    trace!(target: "middleware::cartridge::vrf", %method, "Intercepting JSON-RPC method.");
                    if let Ok(params) = parse_params::<AddExecuteOutsideRequestParams>(&request)
                        .inspect_err(|error| debug!(target: "middleware::cartridge::vrf", %method, %error, "Failed to parse params."))
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

fn find_and_get_request_random_call(outside_execution: &OutsideExecution) -> Option<(Call, usize)> {
    let calls = outside_execution.calls();
    calls
        .iter()
        .position(|call| call.entry_point_selector == selector!("request_random"))
        .map(|position| (calls[position].clone(), position))
}

#[cfg(test)]
mod tests {
    use katana_primitives::{felt, ContractAddress};
    use katana_rpc_types::outside_execution::OutsideExecutionV2;

    use super::*;

    const ANY_CALLER: Felt = felt!("0x414e595f43414c4c4552");

    #[test]
    fn request_random_call_finds_position() {
        let vrf_address = ContractAddress::from(felt!("0x123"));

        let other_call = Call {
            calldata: vec![Felt::ONE],
            contract_address: vrf_address,
            entry_point_selector: selector!("transfer"),
        };

        let vrf_call = Call {
            calldata: vec![Felt::TWO],
            contract_address: vrf_address,
            entry_point_selector: selector!("request_random"),
        };

        let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
            caller: ContractAddress::from(ANY_CALLER),
            execute_after: 0,
            execute_before: 100,
            calls: vec![other_call.clone(), vrf_call.clone()],
            nonce: Felt::THREE,
        });

        let (call, position) =
            find_and_get_request_random_call(&outside_execution).expect("request_random found");

        assert_eq!(position, 1);
        assert_eq!(call.entry_point_selector, vrf_call.entry_point_selector);
        assert_eq!(call.calldata, vrf_call.calldata);
    }
}
