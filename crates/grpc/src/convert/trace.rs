//! Trace type conversions.

use katana_primitives::Felt;

use super::FeltVecExt;
use crate::protos::starknet::{
    SimulateTransactionsResponse, TraceBlockTransactionsResponse, TraceTransactionResponse,
    TransactionTraceWithHash,
};
use crate::protos::types::transaction_trace::Trace as ProtoTraceVariant;
use crate::protos::types::{
    ComputationResources, DataAvailability, DeclareTransactionTrace, DeployAccountTransactionTrace,
    Event as ProtoEvent, ExecutionResources, FeeEstimate as ProtoFeeEstimate, Felt as ProtoFelt,
    FunctionInvocation, InvokeTransactionTrace, L1HandlerTransactionTrace,
    MessageToL1 as ProtoMessageToL1, SimulatedTransaction,
    TransactionTrace as ProtoTransactionTrace,
};

/// Convert RPC trace to proto.
impl From<&katana_rpc_types::trace::TxTrace> for ProtoTransactionTrace {
    fn from(trace: &katana_rpc_types::trace::TxTrace) -> Self {
        use katana_rpc_types::trace::TxTrace;

        let trace_variant = match trace {
            TxTrace::Invoke(invoke) => ProtoTraceVariant::InvokeTrace(InvokeTransactionTrace {
                execute_invocation: invoke.execute_invocation.as_ref().map(|inv| match inv {
                    katana_rpc_types::trace::ExecuteInvocation::Success(inv) => {
                        FunctionInvocation::from(inv)
                    }
                    katana_rpc_types::trace::ExecuteInvocation::Reverted(r) => {
                        // For reverted executions, we create an empty invocation with the revert
                        // reason
                        FunctionInvocation {
                            result: vec![ProtoFelt::from(Felt::from_bytes_be_slice(
                                r.revert_reason.as_bytes(),
                            ))],
                            ..Default::default()
                        }
                    }
                }),
                validate_invocation: invoke
                    .validate_invocation
                    .as_ref()
                    .map(FunctionInvocation::from),
                fee_transfer_invocation: invoke
                    .fee_transfer_invocation
                    .as_ref()
                    .map(FunctionInvocation::from),
                state_diff: String::new(), // State diff is complex, simplified for now
                execution_resources: invoke
                    .execution_resources
                    .as_ref()
                    .map(ExecutionResources::from),
            }),
            TxTrace::Declare(declare) => ProtoTraceVariant::DeclareTrace(DeclareTransactionTrace {
                validate_invocation: declare
                    .validate_invocation
                    .as_ref()
                    .map(FunctionInvocation::from),
                fee_transfer_invocation: declare
                    .fee_transfer_invocation
                    .as_ref()
                    .map(FunctionInvocation::from),
                state_diff: String::new(),
                execution_resources: declare
                    .execution_resources
                    .as_ref()
                    .map(ExecutionResources::from),
            }),
            TxTrace::DeployAccount(deploy) => {
                ProtoTraceVariant::DeployAccountTrace(DeployAccountTransactionTrace {
                    constructor_invocation: deploy
                        .constructor_invocation
                        .as_ref()
                        .map(FunctionInvocation::from),
                    validate_invocation: deploy
                        .validate_invocation
                        .as_ref()
                        .map(FunctionInvocation::from),
                    fee_transfer_invocation: deploy
                        .fee_transfer_invocation
                        .as_ref()
                        .map(FunctionInvocation::from),
                    state_diff: String::new(),
                    execution_resources: deploy
                        .execution_resources
                        .as_ref()
                        .map(ExecutionResources::from),
                })
            }
            TxTrace::L1Handler(l1) => {
                ProtoTraceVariant::L1HandlerTrace(L1HandlerTransactionTrace {
                    function_invocation: l1
                        .function_invocation
                        .as_ref()
                        .map(FunctionInvocation::from),
                    state_diff: String::new(),
                    execution_resources: l1
                        .execution_resources
                        .as_ref()
                        .map(ExecutionResources::from),
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
            entry_point_type: inv.entry_point_type.to_string(),
            call_type: inv.call_type.to_string(),
            result: inv.result.to_proto_felts(),
            calls: inv.calls.iter().map(FunctionInvocation::from).collect(),
            events: inv
                .events
                .iter()
                .map(|e| ProtoEvent {
                    from_address: None, // Events in traces don't have from_address
                    keys: e.keys.to_proto_felts(),
                    data: e.data.to_proto_felts(),
                })
                .collect(),
            messages: inv
                .messages
                .iter()
                .map(|m| ProtoMessageToL1 {
                    from_address: None,
                    to_address: Some(m.to_address.into()),
                    payload: m.payload.to_proto_felts(),
                })
                .collect(),
            computation_resources: Some(ComputationResources {
                steps: inv.computation_resources.steps,
                memory_holes: inv.computation_resources.memory_holes.unwrap_or(0),
                range_check_builtin_applications: inv
                    .computation_resources
                    .range_check_builtin_applications
                    .unwrap_or(0),
                pedersen_builtin_applications: inv
                    .computation_resources
                    .pedersen_builtin_applications
                    .unwrap_or(0),
                poseidon_builtin_applications: inv
                    .computation_resources
                    .poseidon_builtin_applications
                    .unwrap_or(0),
                ec_op_builtin_applications: inv
                    .computation_resources
                    .ec_op_builtin_applications
                    .unwrap_or(0),
                ecdsa_builtin_applications: inv
                    .computation_resources
                    .ecdsa_builtin_applications
                    .unwrap_or(0),
                bitwise_builtin_applications: inv
                    .computation_resources
                    .bitwise_builtin_applications
                    .unwrap_or(0),
                keccak_builtin_applications: inv
                    .computation_resources
                    .keccak_builtin_applications
                    .unwrap_or(0),
                segment_arena_builtin: inv.computation_resources.segment_arena_builtin.unwrap_or(0),
            }),
        }
    }
}

impl From<&katana_rpc_types::trace::ExecutionResources> for ExecutionResources {
    fn from(resources: &katana_rpc_types::trace::ExecutionResources) -> Self {
        ExecutionResources {
            steps: resources.computation_resources.steps,
            memory_holes: resources.computation_resources.memory_holes.unwrap_or(0),
            range_check_builtin_applications: resources
                .computation_resources
                .range_check_builtin_applications
                .unwrap_or(0),
            pedersen_builtin_applications: resources
                .computation_resources
                .pedersen_builtin_applications
                .unwrap_or(0),
            poseidon_builtin_applications: resources
                .computation_resources
                .poseidon_builtin_applications
                .unwrap_or(0),
            ec_op_builtin_applications: resources
                .computation_resources
                .ec_op_builtin_applications
                .unwrap_or(0),
            ecdsa_builtin_applications: resources
                .computation_resources
                .ecdsa_builtin_applications
                .unwrap_or(0),
            bitwise_builtin_applications: resources
                .computation_resources
                .bitwise_builtin_applications
                .unwrap_or(0),
            keccak_builtin_applications: resources
                .computation_resources
                .keccak_builtin_applications
                .unwrap_or(0),
            segment_arena_builtin: resources
                .computation_resources
                .segment_arena_builtin
                .unwrap_or(0),
            data_availability: Some(DataAvailability {
                l1_gas: resources.data_availability.l1_gas,
                l1_data_gas: resources.data_availability.l1_data_gas,
            }),
        }
    }
}

impl From<&katana_rpc_types::FeeEstimate> for ProtoFeeEstimate {
    fn from(estimate: &katana_rpc_types::FeeEstimate) -> Self {
        ProtoFeeEstimate {
            gas_consumed: Some(estimate.l1_gas_consumed.into()),
            gas_price: Some(estimate.l1_gas_price.into()),
            data_gas_consumed: Some(estimate.l1_data_gas_consumed.into()),
            data_gas_price: Some(estimate.l1_data_gas_price.into()),
            overall_fee: Some(estimate.overall_fee.into()),
            unit: estimate.unit.to_string(),
        }
    }
}
