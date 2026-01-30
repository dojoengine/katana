//! Receipt type conversions.

use katana_primitives::block::FinalityStatus;
use katana_primitives::fee::PriceUnit;
use katana_primitives::receipt::{Event, MessageToL1};
use katana_primitives::Felt;

use super::FeltVecExt;
use crate::protos::types::{
    Event as ProtoEvent, ExecutionResources as ProtoExecutionResources,
    FeePayment as ProtoFeePayment, Felt as ProtoFelt, MessageToL1 as ProtoMessageToL1,
    Transaction as ProtoTx, TransactionReceipt as ProtoTransactionReceipt, TransactionWithReceipt,
};

/// Convert PriceUnit to string representation for proto.
fn price_unit_to_string(unit: PriceUnit) -> String {
    match unit {
        PriceUnit::Wei => "WEI".to_string(),
        PriceUnit::Fri => "FRI".to_string(),
    }
}

/// Convert FinalityStatus to string representation for proto.
fn finality_status_to_string(status: &FinalityStatus) -> String {
    match status {
        FinalityStatus::AcceptedOnL1 => "ACCEPTED_ON_L1".to_string(),
        FinalityStatus::AcceptedOnL2 => "ACCEPTED_ON_L2".to_string(),
        FinalityStatus::PreConfirmed => "PRE_CONFIRMED".to_string(),
    }
}

/// Convert RPC transaction with receipt to proto.
impl From<katana_rpc_types::block::RpcTxWithReceipt> for TransactionWithReceipt {
    fn from(tx_with_receipt: katana_rpc_types::block::RpcTxWithReceipt) -> Self {
        TransactionWithReceipt {
            transaction: Some(ProtoTx::from(katana_rpc_types::transaction::RpcTxWithHash {
                transaction_hash: tx_with_receipt.receipt.transaction_hash,
                transaction: tx_with_receipt.transaction,
            })),
            receipt: Some(ProtoTransactionReceipt::from(&tx_with_receipt.receipt)),
        }
    }
}

/// Convert RPC receipt with block info to proto.
impl From<&katana_rpc_types::receipt::TxReceiptWithBlockInfo> for ProtoTransactionReceipt {
    fn from(receipt: &katana_rpc_types::receipt::TxReceiptWithBlockInfo) -> Self {
        let mut proto = receipt_from_rpc_receipt(receipt.transaction_hash, &receipt.receipt);
        // TxReceiptWithBlockInfo has block info via the block field
        proto.block_hash = receipt.block.block_hash().map(|h| h.into());
        proto.block_number = receipt.block.block_number();
        proto
    }
}

/// Convert RPC receipt with hash to proto.
impl From<&katana_rpc_types::receipt::RpcTxReceiptWithHash> for ProtoTransactionReceipt {
    fn from(receipt: &katana_rpc_types::receipt::RpcTxReceiptWithHash) -> Self {
        receipt_from_rpc_receipt(receipt.transaction_hash, &receipt.receipt)
    }
}

fn receipt_from_rpc_receipt(
    transaction_hash: Felt,
    receipt: &katana_rpc_types::receipt::RpcTxReceipt,
) -> ProtoTransactionReceipt {
    use katana_rpc_types::receipt::RpcTxReceipt;

    let tx_type = match receipt {
        RpcTxReceipt::Invoke(_) => "INVOKE",
        RpcTxReceipt::Declare(_) => "DECLARE",
        RpcTxReceipt::Deploy(_) => "DEPLOY",
        RpcTxReceipt::DeployAccount(_) => "DEPLOY_ACCOUNT",
        RpcTxReceipt::L1Handler(_) => "L1_HANDLER",
    };

    let (execution_status, revert_reason) = match receipt.execution_result() {
        katana_rpc_types::receipt::ExecutionResult::Succeeded => {
            ("SUCCEEDED".to_string(), String::new())
        }
        katana_rpc_types::receipt::ExecutionResult::Reverted { reason } => {
            ("REVERTED".to_string(), reason.clone())
        }
    };

    // Extract type-specific fields
    let (contract_address, message_hash) = match receipt {
        RpcTxReceipt::Deploy(r) => (Some(ProtoFelt::from(Felt::from(r.contract_address))), None),
        RpcTxReceipt::DeployAccount(r) => {
            (Some(ProtoFelt::from(Felt::from(r.contract_address))), None)
        }
        RpcTxReceipt::L1Handler(r) => {
            // Convert B256 message_hash to Felt
            let hash_bytes = r.message_hash.0;
            let hash_felt = Felt::from_bytes_be_slice(&hash_bytes);
            (None, Some(hash_felt.into()))
        }
        _ => (None, None),
    };

    ProtoTransactionReceipt {
        r#type: tx_type.to_string(),
        transaction_hash: Some(transaction_hash.into()),
        actual_fee: Some(ProtoFeePayment {
            amount: Some(receipt.actual_fee().amount.into()),
            unit: price_unit_to_string(receipt.actual_fee().unit),
        }),
        finality_status: finality_status_to_string(receipt.finality_status()),
        messages_sent: messages_to_proto(receipt.messages_sent()),
        events: events_to_proto(receipt.events()),
        execution_resources: Some(ProtoExecutionResources::from(receipt.execution_resources())),
        execution_status,
        revert_reason,
        contract_address,
        message_hash,
        block_number: 0,  // Will be set by caller if available
        block_hash: None, // Will be set by caller if available
    }
}

fn messages_to_proto(messages: &[MessageToL1]) -> Vec<ProtoMessageToL1> {
    messages
        .iter()
        .map(|m| ProtoMessageToL1 {
            from_address: Some(ProtoFelt::from(Felt::from(m.from_address))),
            to_address: Some(m.to_address.into()),
            payload: m.payload.to_proto_felts(),
        })
        .collect()
}

fn events_to_proto(events: &[Event]) -> Vec<ProtoEvent> {
    events
        .iter()
        .map(|e| ProtoEvent {
            from_address: Some(ProtoFelt::from(Felt::from(e.from_address))),
            keys: e.keys.to_proto_felts(),
            data: e.data.to_proto_felts(),
        })
        .collect()
}

// Note: ExecutionResources conversion is in trace.rs since it's the same type
// re-exported in katana_rpc_types::receipt and katana_rpc_types::trace
