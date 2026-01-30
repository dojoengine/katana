//! Receipt type conversions.

use super::{to_proto_felt, to_proto_felts, to_proto_transaction};
use crate::protos::types::{
    DataAvailability, Event as ProtoEvent, ExecutionResources as ProtoExecutionResources,
    FeePayment, MessageToL1 as ProtoMessageToL1,
    TransactionReceipt as ProtoTransactionReceipt, TransactionWithReceipt,
};

/// Converts an RPC transaction with receipt to proto.
pub fn to_proto_tx_with_receipt(
    tx_with_receipt: katana_rpc_types::block::TxWithReceipt,
) -> TransactionWithReceipt {
    TransactionWithReceipt {
        transaction: Some(to_proto_transaction(tx_with_receipt.transaction)),
        receipt: Some(to_proto_receipt_inner(&tx_with_receipt.receipt)),
    }
}

/// Converts an RPC receipt with block info to proto.
pub fn to_proto_receipt(
    receipt: &katana_rpc_types::receipt::TxReceiptWithBlockInfo,
) -> ProtoTransactionReceipt {
    use katana_rpc_types::receipt::TxReceiptWithBlockInfo;

    match receipt {
        TxReceiptWithBlockInfo::Invoke(r) => to_proto_receipt_common(
            "INVOKE",
            r.common.transaction_hash,
            &r.common.actual_fee,
            &r.common.finality_status,
            &r.common.messages_sent,
            &r.common.events,
            &r.common.execution_resources,
            r.common.execution_result.clone(),
        ),
        TxReceiptWithBlockInfo::Declare(r) => to_proto_receipt_common(
            "DECLARE",
            r.common.transaction_hash,
            &r.common.actual_fee,
            &r.common.finality_status,
            &r.common.messages_sent,
            &r.common.events,
            &r.common.execution_resources,
            r.common.execution_result.clone(),
        ),
        TxReceiptWithBlockInfo::DeployAccount(r) => to_proto_receipt_common(
            "DEPLOY_ACCOUNT",
            r.common.transaction_hash,
            &r.common.actual_fee,
            &r.common.finality_status,
            &r.common.messages_sent,
            &r.common.events,
            &r.common.execution_resources,
            r.common.execution_result.clone(),
        ),
        TxReceiptWithBlockInfo::L1Handler(r) => to_proto_receipt_common(
            "L1_HANDLER",
            r.common.transaction_hash,
            &r.common.actual_fee,
            &r.common.finality_status,
            &r.common.messages_sent,
            &r.common.events,
            &r.common.execution_resources,
            r.common.execution_result.clone(),
        ),
    }
}

fn to_proto_receipt_inner(
    receipt: &katana_rpc_types::receipt::TxReceipt,
) -> ProtoTransactionReceipt {
    use katana_rpc_types::receipt::TxReceipt;

    match receipt {
        TxReceipt::Invoke(r) => to_proto_receipt_common(
            "INVOKE",
            r.common.transaction_hash,
            &r.common.actual_fee,
            &r.common.finality_status,
            &r.common.messages_sent,
            &r.common.events,
            &r.common.execution_resources,
            r.common.execution_result.clone(),
        ),
        TxReceipt::Declare(r) => to_proto_receipt_common(
            "DECLARE",
            r.common.transaction_hash,
            &r.common.actual_fee,
            &r.common.finality_status,
            &r.common.messages_sent,
            &r.common.events,
            &r.common.execution_resources,
            r.common.execution_result.clone(),
        ),
        TxReceipt::DeployAccount(r) => to_proto_receipt_common(
            "DEPLOY_ACCOUNT",
            r.common.transaction_hash,
            &r.common.actual_fee,
            &r.common.finality_status,
            &r.common.messages_sent,
            &r.common.events,
            &r.common.execution_resources,
            r.common.execution_result.clone(),
        ),
        TxReceipt::L1Handler(r) => to_proto_receipt_common(
            "L1_HANDLER",
            r.common.transaction_hash,
            &r.common.actual_fee,
            &r.common.finality_status,
            &r.common.messages_sent,
            &r.common.events,
            &r.common.execution_resources,
            r.common.execution_result.clone(),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn to_proto_receipt_common(
    tx_type: &str,
    transaction_hash: katana_primitives::Felt,
    actual_fee: &katana_rpc_types::receipt::FeePayment,
    finality_status: &katana_rpc_types::FinalityStatus,
    messages_sent: &[katana_rpc_types::receipt::MsgToL1],
    events: &[katana_rpc_types::receipt::Event],
    execution_resources: &katana_rpc_types::receipt::ExecutionResources,
    execution_result: katana_rpc_types::ExecutionResult,
) -> ProtoTransactionReceipt {
    let (execution_status, revert_reason) = match execution_result {
        katana_rpc_types::ExecutionResult::Succeeded => ("SUCCEEDED".to_string(), String::new()),
        katana_rpc_types::ExecutionResult::Reverted { reason } => ("REVERTED".to_string(), reason),
    };

    ProtoTransactionReceipt {
        r#type: tx_type.to_string(),
        transaction_hash: Some(to_proto_felt(transaction_hash)),
        actual_fee: Some(FeePayment {
            amount: Some(to_proto_felt(actual_fee.amount)),
            unit: actual_fee.unit.to_string(),
        }),
        finality_status: finality_status.to_string(),
        messages_sent: messages_sent
            .iter()
            .map(|m| ProtoMessageToL1 {
                from_address: Some(to_proto_felt(m.from_address.into())),
                to_address: Some(to_proto_felt(m.to_address)),
                payload: to_proto_felts(&m.payload),
            })
            .collect(),
        events: events
            .iter()
            .map(|e| ProtoEvent {
                from_address: Some(to_proto_felt(e.from_address.into())),
                keys: to_proto_felts(&e.keys),
                data: to_proto_felts(&e.data),
            })
            .collect(),
        execution_resources: Some(to_proto_execution_resources(execution_resources)),
        execution_status,
        revert_reason,
    }
}

fn to_proto_execution_resources(
    resources: &katana_rpc_types::receipt::ExecutionResources,
) -> ProtoExecutionResources {
    ProtoExecutionResources {
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
