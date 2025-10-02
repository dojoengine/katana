use std::sync::Arc;

use katana_primitives::class::ClassHash;
use katana_primitives::execution::{
    self, CallInfo, EntryPointSelector, EntryPointType, TrackedResource, TransactionExecutionInfo,
};
use katana_primitives::fee::FeeInfo;
use katana_primitives::transaction::TxType;
use katana_primitives::{receipt, ContractAddress, Felt};
use serde::{Deserialize, Serialize};

use crate::state_update::StateDiff;
use crate::{ExecutionResources, FeeEstimate};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TraceBlockTransactionsResponse {
    pub traces: Vec<TxTraceWithHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SimulatedTransactionsResponse {
    pub transactions: Vec<SimulatedTransactions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatedTransactions {
    /// The transaction's trace
    pub transaction_trace: TxTrace,
    /// The transaction's resources and fee
    pub fee_estimation: FeeEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxTraceWithHash {
    pub transaction_hash: Felt,
    pub trace_root: TxTrace,
}

/// Execution trace of a Starknet transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TxTrace {
    #[serde(rename = "INVOKE")]
    Invoke(InvokeTxTrace),

    #[serde(rename = "DECLARE")]
    Declare(DeclareTxTrace),

    #[serde(rename = "L1_HANDLER")]
    L1Handler(L1HandlerTxTrace),

    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(Box<DeployAccountTxTrace>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallType {
    #[serde(rename = "LIBRARY_CALL")]
    LibraryCall,

    #[serde(rename = "CALL")]
    Call,

    #[serde(rename = "DELEGATE")]
    Delegate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedEvent {
    pub order: u64,
    pub keys: Vec<Felt>,
    pub data: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedL2ToL1Message {
    pub order: u64,
    pub from_address: ContractAddress,
    pub to_address: Felt,
    pub payload: Vec<Felt>,
}

/// Execution resources.
///
/// The resources consumed by an inner call (does not account for state diffs since data is squashed
/// across the transaction).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InnerCallExecutionResources {
    /// L1 gas consumed by this transaction, used for L2-->L1 messages and state updates if blobs
    /// are not used
    pub l1_gas: u64,
    /// L2 gas consumed by this transaction, used for computation and calldata
    pub l2_gas: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionInvocation {
    /// Contract address
    pub contract_address: ContractAddress,
    /// Entry point selector
    pub entry_point_selector: EntryPointSelector,
    /// The parameters passed to the function
    pub calldata: Vec<Felt>,
    /// The address of the invoking contract. 0 for the root invocation
    pub caller_address: ContractAddress,
    /// The hash of the class being called
    pub class_hash: ClassHash,
    pub entry_point_type: EntryPointType,
    pub call_type: CallType,
    /// The value returned from the function invocation
    pub result: Vec<Felt>,
    /// The calls made by this invocation
    pub calls: Vec<FunctionInvocation>,
    /// The events emitted in this invocation
    pub events: Vec<OrderedEvent>,
    /// The messages sent by this invocation to L1
    pub messages: Vec<OrderedL2ToL1Message>,
    /// Resources consumed by the call tree rooted at this given call (including the root)
    pub execution_resources: InnerCallExecutionResources,
    /// True if this inner call panicked
    pub is_reverted: bool,
}

/// The execution result of a function invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExecuteInvocation {
    /// Successful invocation.
    Success(Box<FunctionInvocation>),
    /// Failed and reverted invocation.
    Reverted(RevertedInvocation),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevertedInvocation {
    /// The revert reason for the failed invocation
    pub revert_reason: String,
}

/// The execution trace of an invoke transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTxTrace {
    pub execute_invocation: ExecuteInvocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_invocation: Option<FunctionInvocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_transfer_invocation: Option<FunctionInvocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_diff: Option<StateDiff>,
    pub execution_resources: ExecutionResources,
}

/// The execution trace of a deploy account transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployAccountTxTrace {
    pub constructor_invocation: FunctionInvocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_invocation: Option<FunctionInvocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_transfer_invocation: Option<FunctionInvocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_diff: Option<StateDiff>,
    pub execution_resources: ExecutionResources,
}

/// The execution trace of an L1 handler transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1HandlerTxTrace {
    pub function_invocation: ExecuteInvocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_diff: Option<StateDiff>,
    pub execution_resources: ExecutionResources,
}

/// The execution trace of a declare transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareTxTrace {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_invocation: Option<FunctionInvocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_transfer_invocation: Option<FunctionInvocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_diff: Option<StateDiff>,
    pub execution_resources: ExecutionResources,
}

impl From<katana_primitives::execution::TypedTransactionExecutionInfo> for TxTrace {
    fn from(trace: katana_primitives::execution::TypedTransactionExecutionInfo) -> Self {
        let tx_type = trace.r#type();
        let trace: TransactionExecutionInfo = trace.into();

        let execution_resources = to_rpc_resources(trace.receipt);
        let fee_transfer_invocation = trace.fee_transfer_call_info.map(Into::into);
        let validate_invocation = trace.validate_call_info.map(Into::into);
        let execute_invocation = trace.execute_call_info.map(Into::into);
        let revert_reason = trace.revert_error.map(|e| e.to_string());
        let state_diff = None; // TODO: compute the state diff

        match tx_type {
            TxType::Invoke => {
                let execute_invocation = if let Some(revert_reason) = revert_reason {
                    let invocation = RevertedInvocation { revert_reason };
                    ExecuteInvocation::Reverted(invocation)
                } else {
                    let invocation = execute_invocation.expect("should exist if not reverted");
                    ExecuteInvocation::Success(Box::new(invocation))
                };

                TxTrace::Invoke(InvokeTxTrace {
                    fee_transfer_invocation,
                    execution_resources,
                    validate_invocation,
                    execute_invocation,
                    state_diff,
                })
            }

            TxType::Declare => TxTrace::Declare(DeclareTxTrace {
                fee_transfer_invocation,
                validate_invocation,
                execution_resources,
                state_diff,
            }),

            TxType::DeployAccount => {
                let constructor_invocation =
                    execute_invocation.expect("should exist if not reverted");
                TxTrace::DeployAccount(Box::new(DeployAccountTxTrace {
                    fee_transfer_invocation,
                    constructor_invocation,
                    validate_invocation,
                    execution_resources,
                    state_diff,
                }))
            }

            TxType::L1Handler => {
                let function_invocation = if let Some(revert_reason) = revert_reason {
                    let invocation = RevertedInvocation { revert_reason };
                    ExecuteInvocation::Reverted(invocation)
                } else {
                    let invocation = execute_invocation.expect("should exist if not reverted");
                    ExecuteInvocation::Success(Box::new(invocation))
                };

                TxTrace::L1Handler(L1HandlerTxTrace {
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
}

impl From<katana_primitives::execution::CallInfo> for FunctionInvocation {
    fn from(value: CallInfo) -> Self {
        // The address of the contract that was called.
        let contract_address = value.call.storage_address.into();

        let entry_point_type = value.call.entry_point_type;
        let result = value.execution.retdata.0;
        let is_reverted = value.execution.failed;
        let caller_address = value.call.caller_address.into();
        let calldata = Arc::unwrap_or_clone(value.call.calldata.0);
        let entry_point_selector = value.call.entry_point_selector.0;
        // See <https://github.com/starkware-libs/blockifier/blob/cb464f5ac2ada88f2844d9f7d62bd6732ceb5b2c/crates/blockifier/src/execution/call_info.rs#L220>
        let class_hash = value.call.class_hash.expect("Class hash mut be set after execution").0;

        let calls = value.inner_calls.into_iter().map(Into::into).collect();
        let events = value.execution.events.into_iter().map(Into::into).collect();

        let call_type = match value.call.call_type {
            execution::CallType::Call => CallType::Call,
            execution::CallType::Delegate => CallType::Delegate,
        };

        let messages = value
            .execution
            .l2_to_l1_messages
            .into_iter()
            .map(|m| OrderedL2ToL1Message {
                order: m.order as u64,
                payload: m.message.payload.0,
                from_address: contract_address,
                to_address: m.message.to_address,
            })
            .collect();

        let execution_resources =
            to_inner_execution_resources(value.tracked_resource, value.execution.gas_consumed);

        Self {
            calls,
            events,
            result,
            messages,
            calldata,
            call_type,
            class_hash,
            is_reverted,
            caller_address,
            contract_address,
            entry_point_type,
            execution_resources,
            entry_point_selector,
        }
    }
}

impl From<katana_primitives::execution::OrderedEvent> for OrderedEvent {
    fn from(event: katana_primitives::execution::OrderedEvent) -> Self {
        Self {
            order: event.order as u64,
            data: event.event.data.0,
            keys: event.event.keys.into_iter().map(|k| k.0).collect(),
        }
    }
}

pub fn to_rpc_fee_estimate(resources: &receipt::ExecutionResources, fee: &FeeInfo) -> FeeEstimate {
    FeeEstimate {
        overall_fee: fee.overall_fee,
        l2_gas_price: fee.l2_gas_price,
        l1_gas_price: fee.l1_gas_price,
        l1_data_gas_price: fee.l1_data_gas_price,
        l1_gas_consumed: resources.gas.l1_gas,
        l2_gas_consumed: resources.gas.l2_gas,
        l1_data_gas_consumed: resources.gas.l1_data_gas,
    }
}

fn to_rpc_resources(receipt: execution::TransactionReceipt) -> ExecutionResources {
    ExecutionResources {
        l2_gas: receipt.gas.l2_gas.0,
        l1_gas: receipt.gas.l1_gas.0,
        l1_data_gas: receipt.gas.l1_data_gas.0,
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
            InnerCallExecutionResources { l1_gas: 0, l2_gas }
        }
    }
}
