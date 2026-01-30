//! Receipt type conversions.

use katana_primitives::Felt;

use super::FeltVecExt;
use crate::protos::types::{
    DataAvailability, Event as ProtoEvent, ExecutionResources as ProtoExecutionResources,
    FeePayment, Felt as ProtoFelt, MessageToL1 as ProtoMessageToL1, Transaction as ProtoTx,
    TransactionReceipt as ProtoTransactionReceipt, TransactionWithReceipt,
};

/// Convert RPC transaction with receipt to proto.
impl From<katana_rpc_types::block::TxWithReceipt> for TransactionWithReceipt {
    fn from(tx_with_receipt: katana_rpc_types::block::TxWithReceipt) -> Self {
        TransactionWithReceipt {
            transaction: Some(ProtoTx::from(tx_with_receipt.transaction)),
            receipt: Some(ProtoTransactionReceipt::from(&tx_with_receipt.receipt)),
        }
    }
}

/// Convert RPC receipt with block info to proto.
impl From<&katana_rpc_types::receipt::TxReceiptWithBlockInfo> for ProtoTransactionReceipt {
    fn from(receipt: &katana_rpc_types::receipt::TxReceiptWithBlockInfo) -> Self {
        use katana_rpc_types::receipt::TxReceiptWithBlockInfo;

        match receipt {
            TxReceiptWithBlockInfo::Invoke(r) => receipt_from_common(
                "INVOKE",
                r.common.transaction_hash,
                &r.common.actual_fee,
                &r.common.finality_status,
                &r.common.messages_sent,
                &r.common.events,
                &r.common.execution_resources,
                r.common.execution_result.clone(),
            ),
            TxReceiptWithBlockInfo::Declare(r) => receipt_from_common(
                "DECLARE",
                r.common.transaction_hash,
                &r.common.actual_fee,
                &r.common.finality_status,
                &r.common.messages_sent,
                &r.common.events,
                &r.common.execution_resources,
                r.common.execution_result.clone(),
            ),
            TxReceiptWithBlockInfo::DeployAccount(r) => receipt_from_common(
                "DEPLOY_ACCOUNT",
                r.common.transaction_hash,
                &r.common.actual_fee,
                &r.common.finality_status,
                &r.common.messages_sent,
                &r.common.events,
                &r.common.execution_resources,
                r.common.execution_result.clone(),
            ),
            TxReceiptWithBlockInfo::L1Handler(r) => receipt_from_common(
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
}

impl From<&katana_rpc_types::receipt::TxReceipt> for ProtoTransactionReceipt {
    fn from(receipt: &katana_rpc_types::receipt::TxReceipt) -> Self {
        use katana_rpc_types::receipt::TxReceipt;

        match receipt {
            TxReceipt::Invoke(r) => receipt_from_common(
                "INVOKE",
                r.common.transaction_hash,
                &r.common.actual_fee,
                &r.common.finality_status,
                &r.common.messages_sent,
                &r.common.events,
                &r.common.execution_resources,
                r.common.execution_result.clone(),
            ),
            TxReceipt::Declare(r) => receipt_from_common(
                "DECLARE",
                r.common.transaction_hash,
                &r.common.actual_fee,
                &r.common.finality_status,
                &r.common.messages_sent,
                &r.common.events,
                &r.common.execution_resources,
                r.common.execution_result.clone(),
            ),
            TxReceipt::DeployAccount(r) => receipt_from_common(
                "DEPLOY_ACCOUNT",
                r.common.transaction_hash,
                &r.common.actual_fee,
                &r.common.finality_status,
                &r.common.messages_sent,
                &r.common.events,
                &r.common.execution_resources,
                r.common.execution_result.clone(),
            ),
            TxReceipt::L1Handler(r) => receipt_from_common(
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
}

#[allow(clippy::too_many_arguments)]
fn receipt_from_common(
    tx_type: &str,
    transaction_hash: Felt,
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
        transaction_hash: Some(transaction_hash.into()),
        actual_fee: Some(FeePayment {
            amount: Some(actual_fee.amount.into()),
            unit: actual_fee.unit.to_string(),
        }),
        finality_status: finality_status.to_string(),
        messages_sent: messages_sent
            .iter()
            .map(|m| ProtoMessageToL1 {
                from_address: Some(ProtoFelt::from(Felt::from(m.from_address))),
                to_address: Some(m.to_address.into()),
                payload: m.payload.to_proto_felts(),
            })
            .collect(),
        events: events
            .iter()
            .map(|e| ProtoEvent {
                from_address: Some(ProtoFelt::from(Felt::from(e.from_address))),
                keys: e.keys.to_proto_felts(),
                data: e.data.to_proto_felts(),
            })
            .collect(),
        execution_resources: Some(ProtoExecutionResources::from(execution_resources)),
        execution_status,
        revert_reason,
    }
}

impl From<&katana_rpc_types::receipt::ExecutionResources> for ProtoExecutionResources {
    fn from(resources: &katana_rpc_types::receipt::ExecutionResources) -> Self {
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
}
