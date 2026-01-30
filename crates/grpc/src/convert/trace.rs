//! Trace type conversions.

use super::{to_proto_felt, to_proto_felts};
use crate::protos::starknet::{
    SimulateTransactionsResponse, TraceBlockTransactionsResponse, TraceTransactionResponse,
    TransactionTraceWithHash,
};
use crate::protos::types::{
    transaction_trace::Trace as ProtoTraceVariant, ComputationResources, DataAvailability,
    DeclareTransactionTrace, DeployAccountTransactionTrace, Event as ProtoEvent,
    ExecutionResources, FeeEstimate as ProtoFeeEstimate, FunctionInvocation,
    InvokeTransactionTrace, L1HandlerTransactionTrace, MessageToL1 as ProtoMessageToL1,
    SimulatedTransaction, TransactionTrace as ProtoTransactionTrace,
};

/// Converts an RPC trace to proto.
pub fn to_proto_trace(trace: &katana_rpc_types::trace::TxTrace) -> ProtoTransactionTrace {
    use katana_rpc_types::trace::TxTrace;

    let trace_variant = match trace {
        TxTrace::Invoke(invoke) => ProtoTraceVariant::InvokeTrace(InvokeTransactionTrace {
            execute_invocation: invoke
                .execute_invocation
                .as_ref()
                .map(|inv| match inv {
                    katana_rpc_types::trace::ExecuteInvocation::Success(inv) => {
                        to_proto_function_invocation(inv)
                    }
                    katana_rpc_types::trace::ExecuteInvocation::Reverted(r) => {
                        // For reverted executions, we create an empty invocation with the revert
                        // reason
                        FunctionInvocation {
                            result: vec![to_proto_felt(
                                katana_primitives::Felt::from_bytes_be_slice(
                                    r.revert_reason.as_bytes(),
                                ),
                            )],
                            ..Default::default()
                        }
                    }
                }),
            validate_invocation: invoke
                .validate_invocation
                .as_ref()
                .map(to_proto_function_invocation),
            fee_transfer_invocation: invoke
                .fee_transfer_invocation
                .as_ref()
                .map(to_proto_function_invocation),
            state_diff: String::new(), // State diff is complex, simplified for now
            execution_resources: invoke.execution_resources.as_ref().map(to_proto_exec_resources),
        }),
        TxTrace::Declare(declare) => ProtoTraceVariant::DeclareTrace(DeclareTransactionTrace {
            validate_invocation: declare
                .validate_invocation
                .as_ref()
                .map(to_proto_function_invocation),
            fee_transfer_invocation: declare
                .fee_transfer_invocation
                .as_ref()
                .map(to_proto_function_invocation),
            state_diff: String::new(),
            execution_resources: declare
                .execution_resources
                .as_ref()
                .map(to_proto_exec_resources),
        }),
        TxTrace::DeployAccount(deploy) => {
            ProtoTraceVariant::DeployAccountTrace(DeployAccountTransactionTrace {
                constructor_invocation: deploy
                    .constructor_invocation
                    .as_ref()
                    .map(to_proto_function_invocation),
                validate_invocation: deploy
                    .validate_invocation
                    .as_ref()
                    .map(to_proto_function_invocation),
                fee_transfer_invocation: deploy
                    .fee_transfer_invocation
                    .as_ref()
                    .map(to_proto_function_invocation),
                state_diff: String::new(),
                execution_resources: deploy
                    .execution_resources
                    .as_ref()
                    .map(to_proto_exec_resources),
            })
        }
        TxTrace::L1Handler(l1) => ProtoTraceVariant::L1HandlerTrace(L1HandlerTransactionTrace {
            function_invocation: l1.function_invocation.as_ref().map(to_proto_function_invocation),
            state_diff: String::new(),
            execution_resources: l1.execution_resources.as_ref().map(to_proto_exec_resources),
        }),
    };

    ProtoTransactionTrace { trace: Some(trace_variant) }
}

/// Converts trace transaction response to proto.
pub fn to_proto_trace_transaction_response(
    trace: katana_rpc_types::trace::TxTrace,
) -> TraceTransactionResponse {
    TraceTransactionResponse { trace: Some(to_proto_trace(&trace)) }
}

/// Converts simulated transactions response to proto.
pub fn to_proto_simulate_transactions_response(
    response: katana_rpc_types::trace::SimulatedTransactionsResponse,
) -> SimulateTransactionsResponse {
    SimulateTransactionsResponse {
        simulated_transactions: response
            .transactions
            .into_iter()
            .map(|sim| SimulatedTransaction {
                transaction_trace: Some(to_proto_trace(&sim.transaction_trace)),
                fee_estimation: Some(to_proto_fee_estimate(&sim.fee_estimation)),
            })
            .collect(),
    }
}

/// Converts trace block transactions response to proto.
pub fn to_proto_trace_block_transactions_response(
    response: katana_rpc_types::trace::TraceBlockTransactionsResponse,
) -> TraceBlockTransactionsResponse {
    TraceBlockTransactionsResponse {
        traces: response
            .traces
            .into_iter()
            .map(|t| TransactionTraceWithHash {
                transaction_hash: Some(to_proto_felt(t.transaction_hash)),
                trace_root: Some(to_proto_trace(&t.trace_root)),
            })
            .collect(),
    }
}

fn to_proto_function_invocation(
    inv: &katana_rpc_types::trace::FunctionInvocation,
) -> FunctionInvocation {
    FunctionInvocation {
        contract_address: Some(to_proto_felt(inv.contract_address.into())),
        entry_point_selector: Some(to_proto_felt(inv.entry_point_selector)),
        calldata: to_proto_felts(&inv.calldata),
        caller_address: Some(to_proto_felt(inv.caller_address.into())),
        class_hash: Some(to_proto_felt(inv.class_hash)),
        entry_point_type: inv.entry_point_type.to_string(),
        call_type: inv.call_type.to_string(),
        result: to_proto_felts(&inv.result),
        calls: inv.calls.iter().map(to_proto_function_invocation).collect(),
        events: inv
            .events
            .iter()
            .map(|e| ProtoEvent {
                from_address: None, // Events in traces don't have from_address
                keys: to_proto_felts(&e.keys),
                data: to_proto_felts(&e.data),
            })
            .collect(),
        messages: inv
            .messages
            .iter()
            .map(|m| ProtoMessageToL1 {
                from_address: None,
                to_address: Some(to_proto_felt(m.to_address)),
                payload: to_proto_felts(&m.payload),
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

fn to_proto_exec_resources(
    resources: &katana_rpc_types::trace::ExecutionResources,
) -> ExecutionResources {
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
        segment_arena_builtin: resources.computation_resources.segment_arena_builtin.unwrap_or(0),
        data_availability: Some(DataAvailability {
            l1_gas: resources.data_availability.l1_gas,
            l1_data_gas: resources.data_availability.l1_data_gas,
        }),
    }
}

fn to_proto_fee_estimate(estimate: &katana_rpc_types::FeeEstimate) -> ProtoFeeEstimate {
    ProtoFeeEstimate {
        gas_consumed: Some(to_proto_felt(estimate.l1_gas_consumed)),
        gas_price: Some(to_proto_felt(estimate.l1_gas_price)),
        data_gas_consumed: Some(to_proto_felt(estimate.l1_data_gas_consumed)),
        data_gas_price: Some(to_proto_felt(estimate.l1_data_gas_price)),
        overall_fee: Some(to_proto_felt(estimate.overall_fee)),
        unit: estimate.unit.to_string(),
    }
}
