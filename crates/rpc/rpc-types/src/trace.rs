use std::collections::HashMap;
use std::sync::Arc;

use katana_primitives::execution::{
    self, BuiltinName, CallInfo, GasVector, TransactionExecutionInfo,
};
use katana_primitives::fee::TxFeeInfo;
use katana_primitives::transaction::{TxHash, TxType};
use serde::{Deserialize, Serialize};
use starknet::core::types::{
    CallType, ComputationResources, DataAvailabilityResources, DataResources,
    DeclareTransactionTrace, DeployAccountTransactionTrace, EntryPointType, ExecuteInvocation,
    ExecutionResources, FunctionInvocation, InvokeTransactionTrace, L1HandlerTransactionTrace,
    OrderedEvent, OrderedMessage, PriceUnit, RevertedInvocation, TransactionTrace,
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

pub fn to_rpc_trace(trace: TransactionExecutionInfo, tx_type: TxType) -> TransactionTrace {
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

pub fn to_rpc_computation_resources(resources: execution::VmResources) -> ComputationResources {
    let builtins = &resources.builtin_instance_counter;
    ComputationResources {
        steps: resources.n_steps as u64,
        memory_holes: Some(resources.n_memory_holes as u64),
        ecdsa_builtin_applications: get_builtin_count(builtins, BuiltinName::ecdsa),
        ec_op_builtin_applications: get_builtin_count(builtins, BuiltinName::ec_op),
        keccak_builtin_applications: get_builtin_count(builtins, BuiltinName::keccak),
        segment_arena_builtin: get_builtin_count(builtins, BuiltinName::segment_arena),
        bitwise_builtin_applications: get_builtin_count(builtins, BuiltinName::bitwise),
        pedersen_builtin_applications: get_builtin_count(builtins, BuiltinName::pedersen),
        poseidon_builtin_applications: get_builtin_count(builtins, BuiltinName::poseidon),
        range_check_builtin_applications: get_builtin_count(builtins, BuiltinName::range_check),
    }
}

pub fn to_rpc_fee_estimate(fee: TxFeeInfo) -> FeeEstimate {
    FeeEstimate {
        unit: match fee.unit {
            katana_primitives::fee::PriceUnit::Wei => PriceUnit::Wei,
            katana_primitives::fee::PriceUnit::Fri => PriceUnit::Fri,
        },
        gas_price: fee.gas_price.into(),
        overall_fee: fee.overall_fee.into(),
        gas_consumed: fee.gas_consumed.into(),
        data_gas_price: Default::default(),
        data_gas_consumed: Default::default(),
    }
}

fn to_rpc_resources(receipt: execution::TransactionReceipt) -> ExecutionResources {
    let data_resources = to_rpc_data_resources(receipt.da_gas);
    let computation_resources = receipt.resources.computation.vm_resources;
    let computation_resources = to_rpc_computation_resources(computation_resources);
    ExecutionResources { data_resources, computation_resources }
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

    let execution_resources = to_rpc_computation_resources(info.resources);

    FunctionInvocation {
        calls,
        events,
        messages,
        call_type,
        entry_point_type,
        execution_resources,
        result: info.execution.retdata.0,
        caller_address: info.call.caller_address.into(),
        contract_address: info.call.storage_address.into(),
        calldata: Arc::unwrap_or_clone(info.call.calldata.0),
        entry_point_selector: info.call.entry_point_selector.0,
        // See <https://github.com/starkware-libs/blockifier/blob/cb464f5ac2ada88f2844d9f7d62bd6732ceb5b2c/crates/blockifier/src/execution/call_info.rs#L220>
        class_hash: info.call.class_hash.expect("Class hash mut be set after execution").0,
    }
}

fn to_rpc_data_resources(da_gas: GasVector) -> DataResources {
    let l1_gas = da_gas.l1_gas.0;
    let l1_data_gas = da_gas.l1_data_gas.0;
    DataResources { data_availability: DataAvailabilityResources { l1_gas, l1_data_gas } }
}

fn get_builtin_count(
    builtins: &HashMap<BuiltinName, usize>,
    builtin_name: BuiltinName,
) -> Option<u64> {
    builtins.get(&builtin_name).map(|&x| x as u64)
}
