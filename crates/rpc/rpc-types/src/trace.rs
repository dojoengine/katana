use std::sync::Arc;

use katana_primitives::execution::{
    self, CallInfo, TrackedResource, TransactionExecutionInfo, TypedTransactionExecutionInfo,
};
use katana_primitives::fee::{self, FeeInfo};
use katana_primitives::receipt;
use katana_primitives::transaction::{TxHash, TxType};
use serde::{Deserialize, Serialize};
use starknet::core::types::{
    CallType, DeclareTransactionTrace, DeployAccountTransactionTrace, EntryPointType,
    ExecuteInvocation, ExecutionResources, FunctionInvocation, InnerCallExecutionResources,
    InvokeTransactionTrace, L1HandlerTransactionTrace, OrderedEvent, OrderedMessage, PriceUnit,
    RevertedInvocation, TransactionTrace,
};

use crate::FeeEstimate;

/// The type returned by the `saya_getTransactionExecutionsByBlock` RPC method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxExecutionInfo {
    /// The transaction hash.
    pub hash: TxHash,
    /// The transaction execution trace.
    pub trace: TransactionExecutionInfo,
}

pub fn to_rpc_trace(trace: TypedTransactionExecutionInfo) -> TransactionTrace {
    let tx_type = trace.r#type();
    let trace: TransactionExecutionInfo = trace.into();

    let execution_resources = to_rpc_resources(trace.receipt);
    let fee_transfer_invocation = trace.fee_transfer_call_info.map(to_function_invocation);
    let validate_invocation = trace.validate_call_info.map(to_function_invocation);
    let execute_invocation = trace.execute_call_info.map(to_function_invocation);
    let revert_reason = trace.revert_error.map(|e| e.to_string());
    let state_diff = None; // TODO: compute the state diff

    match tx_type {
        TxType::Invoke => {
            let execute_invocation = if let Some(revert_reason) = revert_reason {
                let invocation = RevertedInvocation { revert_reason };
                ExecuteInvocation::Reverted(invocation)
            } else {
                let invocation = execute_invocation.expect("should exist if not reverted");
                ExecuteInvocation::Success(invocation)
            };

            TransactionTrace::Invoke(InvokeTransactionTrace {
                fee_transfer_invocation,
                execution_resources,
                validate_invocation,
                execute_invocation,
                state_diff,
            })
        }

        TxType::Declare => TransactionTrace::Declare(DeclareTransactionTrace {
            fee_transfer_invocation,
            validate_invocation,
            execution_resources,
            state_diff,
        }),

        TxType::DeployAccount => {
            let constructor_invocation = execute_invocation.expect("should exist if not reverted");
            TransactionTrace::DeployAccount(DeployAccountTransactionTrace {
                fee_transfer_invocation,
                constructor_invocation,
                validate_invocation,
                execution_resources,
                state_diff,
            })
        }

        TxType::L1Handler => {
            let function_invocation = execute_invocation.expect("should exist if not reverted");
            TransactionTrace::L1Handler(L1HandlerTransactionTrace {
                execution_resources,
                function_invocation,
                state_diff,
            })
        }

        TxType::Deploy => {
            unimplemented!("unsupported legacy tx type")
        }
    }
}

pub fn to_rpc_fee_estimate(resources: receipt::ExecutionResources, fee: &FeeInfo) -> FeeEstimate {
    let unit = match fee.unit {
        fee::PriceUnit::Wei => PriceUnit::Wei,
        fee::PriceUnit::Fri => PriceUnit::Fri,
    };

    FeeEstimate {
        unit,
        overall_fee: fee.overall_fee.into(),
        l2_gas_price: fee.l2_gas_price.into(),
        l1_gas_price: fee.l1_gas_price.into(),
        l1_data_gas_price: fee.l1_data_gas_price.into(),
        l1_gas_consumed: resources.gas.l1_gas.into(),
        l2_gas_consumed: resources.gas.l2_gas.into(),
        l1_data_gas_consumed: resources.gas.l1_data_gas.into(),
    }
}

fn to_rpc_resources(receipt: execution::TransactionReceipt) -> ExecutionResources {
    ExecutionResources {
        l2_gas: receipt.gas.l2_gas.0,
        l1_gas: receipt.gas.l1_gas.0,
        l1_data_gas: receipt.gas.l1_data_gas.0,
    }
}

fn to_function_invocation(info: CallInfo) -> FunctionInvocation {
    let contract_address = info.call.storage_address;
    let calls = info.inner_calls.into_iter().map(to_function_invocation).collect();

    let entry_point_type = match info.call.entry_point_type {
        execution::EntryPointType::External => EntryPointType::External,
        execution::EntryPointType::L1Handler => EntryPointType::L1Handler,
        execution::EntryPointType::Constructor => EntryPointType::Constructor,
    };

    let call_type = match info.call.call_type {
        execution::CallType::Call => CallType::Call,
        execution::CallType::Delegate => CallType::Delegate,
    };

    let events = info
        .execution
        .events
        .into_iter()
        .map(|e| OrderedEvent {
            order: e.order as u64,
            data: e.event.data.0,
            keys: e.event.keys.into_iter().map(|k| k.0).collect(),
        })
        .collect();

    let messages = info
        .execution
        .l2_to_l1_messages
        .into_iter()
        .map(|m| OrderedMessage {
            order: m.order as u64,
            payload: m.message.payload.0,
            to_address: m.message.to_address,
            from_address: contract_address.into(),
        })
        .collect();

    let execution_resources =
        to_inner_execution_resources(info.tracked_resource, info.execution.gas_consumed);

    FunctionInvocation {
        calls,
        events,
        messages,
        call_type,
        entry_point_type,
        execution_resources,
        result: info.execution.retdata.0,
        is_reverted: info.execution.failed,
        caller_address: info.call.caller_address.into(),
        contract_address: info.call.storage_address.into(),
        calldata: Arc::unwrap_or_clone(info.call.calldata.0),
        entry_point_selector: info.call.entry_point_selector.0,
        // See <https://github.com/starkware-libs/blockifier/blob/cb464f5ac2ada88f2844d9f7d62bd6732ceb5b2c/crates/blockifier/src/execution/call_info.rs#L220>
        class_hash: info.call.class_hash.expect("Class hash mut be set after execution").0,
    }
}

fn to_inner_execution_resources(
    resources: TrackedResource,
    gas_consumed: u64,
) -> InnerCallExecutionResources {
    match resources {
        TrackedResource::CairoSteps => {
            let l1_gas = gas_consumed;
            InnerCallExecutionResources { l1_gas, l2_gas: 0 }
        }
        TrackedResource::SierraGas => {
            let l2_gas = gas_consumed;
            InnerCallExecutionResources { l2_gas, l1_gas: 0 }
        }
    }
}
