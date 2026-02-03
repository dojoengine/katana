//! Trace type conversions.

use katana_primitives::Felt;

use super::FeltVecExt;
use crate::protos::common::Felt as ProtoFelt;
use crate::protos::starknet::{
    SimulateTransactionsResponse, TraceBlockTransactionsResponse, TraceTransactionResponse,
    TransactionTraceWithHash,
};
use crate::protos::types::transaction_trace::Trace as ProtoTraceVariant;
use crate::protos::types::{
    DeclareTransactionTrace, DeployAccountTransactionTrace, ExecutionResources,
    FeeEstimate as ProtoFeeEstimate, FunctionInvocation, InvokeTransactionTrace,
    L1HandlerTransactionTrace, OrderedEvent, OrderedL2ToL1Message, SimulatedTransaction,
    TransactionTrace as ProtoTransactionTrace,
};

/// Convert RPC trace to proto.
impl From<&katana_rpc_types::trace::TxTrace> for ProtoTransactionTrace {
    fn from(trace: &katana_rpc_types::trace::TxTrace) -> Self {
        use katana_rpc_types::trace::TxTrace;

        let trace_variant = match trace {
            TxTrace::Invoke(invoke) => {
                let execute_invocation = match &invoke.execute_invocation {
                    katana_rpc_types::trace::ExecuteInvocation::Success(inv) => {
                        Some(FunctionInvocation::from(inv.as_ref()))
                    }
                    katana_rpc_types::trace::ExecuteInvocation::Reverted(r) => {
                        // For reverted executions, create invocation with revert info
                        Some(FunctionInvocation {
                            is_reverted: true,
                            result: vec![ProtoFelt::from(Felt::from_bytes_be_slice(
                                r.revert_reason.as_bytes(),
                            ))],
                            ..Default::default()
                        })
                    }
                };

                ProtoTraceVariant::InvokeTrace(InvokeTransactionTrace {
                    execute_invocation,
                    validate_invocation: invoke
                        .validate_invocation
                        .as_ref()
                        .map(FunctionInvocation::from),
                    fee_transfer_invocation: invoke
                        .fee_transfer_invocation
                        .as_ref()
                        .map(FunctionInvocation::from),
                    state_diff: None, // State diff conversion would require more complex mapping
                    execution_resources: Some(ExecutionResources::from(
                        &invoke.execution_resources,
                    )),
                })
            }
            TxTrace::Declare(declare) => ProtoTraceVariant::DeclareTrace(DeclareTransactionTrace {
                validate_invocation: declare
                    .validate_invocation
                    .as_ref()
                    .map(FunctionInvocation::from),
                fee_transfer_invocation: declare
                    .fee_transfer_invocation
                    .as_ref()
                    .map(FunctionInvocation::from),
                state_diff: None,
                execution_resources: Some(ExecutionResources::from(&declare.execution_resources)),
            }),
            TxTrace::DeployAccount(deploy) => {
                ProtoTraceVariant::DeployAccountTrace(DeployAccountTransactionTrace {
                    constructor_invocation: Some(FunctionInvocation::from(
                        &deploy.constructor_invocation,
                    )),
                    validate_invocation: deploy
                        .validate_invocation
                        .as_ref()
                        .map(FunctionInvocation::from),
                    fee_transfer_invocation: deploy
                        .fee_transfer_invocation
                        .as_ref()
                        .map(FunctionInvocation::from),
                    state_diff: None,
                    execution_resources: Some(ExecutionResources::from(
                        &deploy.execution_resources,
                    )),
                })
            }
            TxTrace::L1Handler(l1) => {
                let function_invocation = match &l1.function_invocation {
                    katana_rpc_types::trace::ExecuteInvocation::Success(inv) => {
                        Some(FunctionInvocation::from(inv.as_ref()))
                    }
                    katana_rpc_types::trace::ExecuteInvocation::Reverted(r) => {
                        Some(FunctionInvocation {
                            is_reverted: true,
                            result: vec![ProtoFelt::from(Felt::from_bytes_be_slice(
                                r.revert_reason.as_bytes(),
                            ))],
                            ..Default::default()
                        })
                    }
                };

                ProtoTraceVariant::L1HandlerTrace(L1HandlerTransactionTrace {
                    function_invocation,
                    state_diff: None,
                    execution_resources: Some(ExecutionResources::from(&l1.execution_resources)),
                })
            }
        };

        ProtoTransactionTrace { trace: Some(trace_variant) }
    }
}

/// Convert trace transaction response to proto.
impl From<katana_rpc_types::trace::TxTrace> for TraceTransactionResponse {
    fn from(trace: katana_rpc_types::trace::TxTrace) -> Self {
        TraceTransactionResponse { trace: Some(ProtoTransactionTrace::from(&trace)) }
    }
}

/// Convert simulated transactions response to proto.
impl From<katana_rpc_types::trace::SimulatedTransactionsResponse> for SimulateTransactionsResponse {
    fn from(response: katana_rpc_types::trace::SimulatedTransactionsResponse) -> Self {
        SimulateTransactionsResponse {
            simulated_transactions: response
                .transactions
                .into_iter()
                .map(|sim| SimulatedTransaction {
                    transaction_trace: Some(ProtoTransactionTrace::from(&sim.transaction_trace)),
                    fee_estimation: Some(ProtoFeeEstimate::from(&sim.fee_estimation)),
                })
                .collect(),
        }
    }
}

/// Convert trace block transactions response to proto.
impl From<katana_rpc_types::trace::TraceBlockTransactionsResponse>
    for TraceBlockTransactionsResponse
{
    fn from(response: katana_rpc_types::trace::TraceBlockTransactionsResponse) -> Self {
        TraceBlockTransactionsResponse {
            traces: response
                .traces
                .into_iter()
                .map(|t| TransactionTraceWithHash {
                    transaction_hash: Some(t.transaction_hash.into()),
                    trace_root: Some(ProtoTransactionTrace::from(&t.trace_root)),
                })
                .collect(),
        }
    }
}

impl From<&katana_rpc_types::trace::FunctionInvocation> for FunctionInvocation {
    fn from(inv: &katana_rpc_types::trace::FunctionInvocation) -> Self {
        FunctionInvocation {
            contract_address: Some(ProtoFelt::from(Felt::from(inv.contract_address))),
            entry_point_selector: Some(inv.entry_point_selector.into()),
            calldata: inv.calldata.to_proto_felts(),
            caller_address: Some(ProtoFelt::from(Felt::from(inv.caller_address))),
            class_hash: Some(inv.class_hash.into()),
            entry_point_type: format!("{:?}", inv.entry_point_type),
            call_type: format!("{:?}", inv.call_type),
            result: inv.result.to_proto_felts(),
            calls: inv.calls.iter().map(FunctionInvocation::from).collect(),
            events: inv
                .events
                .iter()
                .map(|e| OrderedEvent {
                    order: e.order,
                    keys: e.keys.to_proto_felts(),
                    data: e.data.to_proto_felts(),
                })
                .collect(),
            messages: inv
                .messages
                .iter()
                .map(|m| OrderedL2ToL1Message {
                    order: m.order,
                    from_address: Some(ProtoFelt::from(Felt::from(m.from_address))),
                    to_address: Some(m.to_address.into()),
                    payload: m.payload.to_proto_felts(),
                })
                .collect(),
            execution_resources: Some(ExecutionResources {
                l1_gas: inv.execution_resources.l1_gas,
                l1_data_gas: 0, // InnerCallExecutionResources doesn't have this
                l2_gas: inv.execution_resources.l2_gas,
            }),
            is_reverted: inv.is_reverted,
        }
    }
}

/// Convert ExecutionResources (from receipt/trace) to proto ExecutionResources.
impl From<&katana_rpc_types::receipt::ExecutionResources> for ExecutionResources {
    fn from(resources: &katana_rpc_types::receipt::ExecutionResources) -> Self {
        ExecutionResources {
            l1_gas: resources.l1_gas,
            l1_data_gas: resources.l1_data_gas,
            l2_gas: resources.l2_gas,
        }
    }
}

impl From<&katana_rpc_types::FeeEstimate> for ProtoFeeEstimate {
    fn from(estimate: &katana_rpc_types::FeeEstimate) -> Self {
        ProtoFeeEstimate {
            l1_gas_consumed: Some(ProtoFelt::from(Felt::from(estimate.l1_gas_consumed))),
            l1_gas_price: Some(ProtoFelt::from(Felt::from(estimate.l1_gas_price))),
            l2_gas_consumed: Some(ProtoFelt::from(Felt::from(estimate.l2_gas_consumed))),
            l2_gas_price: Some(ProtoFelt::from(Felt::from(estimate.l2_gas_price))),
            l1_data_gas_consumed: Some(ProtoFelt::from(Felt::from(estimate.l1_data_gas_consumed))),
            l1_data_gas_price: Some(ProtoFelt::from(Felt::from(estimate.l1_data_gas_price))),
            overall_fee: Some(ProtoFelt::from(Felt::from(estimate.overall_fee))),
        }
    }
}
