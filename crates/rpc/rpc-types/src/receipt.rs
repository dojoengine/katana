use katana_primitives::block::FinalityStatus;
use katana_primitives::fee::{FeeInfo, PriceUnit};
use katana_primitives::receipt::{self, MessageToL1, Receipt};
use katana_primitives::transaction::TxHash;
use serde::{Deserialize, Serialize};
pub use starknet::core::types::ReceiptBlock;
use starknet::core::types::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, ExecutionResources,
    ExecutionResult, FeePayment, Hash256, InvokeTransactionReceipt, L1HandlerTransactionReceipt,
    TransactionFinalityStatus, TransactionReceipt, TransactionReceiptWithBlockInfo,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TxReceipt(pub(crate) starknet::core::types::TransactionReceipt);

impl TxReceipt {
    pub fn new(
        transaction_hash: TxHash,
        finality_status: FinalityStatus,
        receipt: Receipt,
    ) -> Self {
        let finality_status = match finality_status {
            FinalityStatus::AcceptedOnL1 => TransactionFinalityStatus::AcceptedOnL1,
            FinalityStatus::AcceptedOnL2 => TransactionFinalityStatus::AcceptedOnL2,
        };

        let receipt = match receipt {
            Receipt::Invoke(rct) => {
                let messages_sent =
                    rct.messages_sent.into_iter().map(|e| MsgToL1::from(e).0).collect();
                let events = rct.events.into_iter().map(|e| Event::from(e).0).collect();

                TransactionReceipt::Invoke(InvokeTransactionReceipt {
                    events,
                    messages_sent,
                    finality_status,
                    transaction_hash,
                    actual_fee: to_rpc_fee(rct.fee),
                    execution_resources: to_rpc_resources(rct.execution_resources),
                    execution_result: if let Some(reason) = rct.revert_error {
                        ExecutionResult::Reverted { reason }
                    } else {
                        ExecutionResult::Succeeded
                    },
                })
            }

            Receipt::Declare(rct) => {
                let messages_sent =
                    rct.messages_sent.into_iter().map(|e| MsgToL1::from(e).0).collect();
                let events = rct.events.into_iter().map(|e| Event::from(e).0).collect();

                TransactionReceipt::Declare(DeclareTransactionReceipt {
                    events,
                    messages_sent,
                    finality_status,
                    transaction_hash,
                    actual_fee: to_rpc_fee(rct.fee),
                    execution_resources: to_rpc_resources(rct.execution_resources),
                    execution_result: if let Some(reason) = rct.revert_error {
                        ExecutionResult::Reverted { reason }
                    } else {
                        ExecutionResult::Succeeded
                    },
                })
            }

            Receipt::L1Handler(rct) => {
                let messages_sent =
                    rct.messages_sent.into_iter().map(|e| MsgToL1::from(e).0).collect();
                let events = rct.events.into_iter().map(|e| Event::from(e).0).collect();

                TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
                    events,
                    messages_sent,
                    finality_status,
                    transaction_hash,
                    actual_fee: to_rpc_fee(rct.fee),
                    execution_resources: to_rpc_resources(rct.execution_resources),
                    message_hash: Hash256::from_bytes(*rct.message_hash),
                    execution_result: if let Some(reason) = rct.revert_error {
                        ExecutionResult::Reverted { reason }
                    } else {
                        ExecutionResult::Succeeded
                    },
                })
            }

            Receipt::DeployAccount(rct) => {
                let messages_sent =
                    rct.messages_sent.into_iter().map(|e| MsgToL1::from(e).0).collect();
                let events = rct.events.into_iter().map(|e| Event::from(e).0).collect();

                TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
                    events,
                    messages_sent,
                    finality_status,
                    transaction_hash,
                    actual_fee: to_rpc_fee(rct.fee),
                    contract_address: rct.contract_address.into(),
                    execution_resources: to_rpc_resources(rct.execution_resources),
                    execution_result: if let Some(reason) = rct.revert_error {
                        ExecutionResult::Reverted { reason }
                    } else {
                        ExecutionResult::Succeeded
                    },
                })
            }
        };

        Self(receipt)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TxReceiptWithBlockInfo(pub starknet::core::types::TransactionReceiptWithBlockInfo);

impl From<starknet::core::types::TransactionReceiptWithBlockInfo> for TxReceiptWithBlockInfo {
    fn from(value: starknet::core::types::TransactionReceiptWithBlockInfo) -> Self {
        Self(value)
    }
}

impl TxReceiptWithBlockInfo {
    pub fn new(
        block: ReceiptBlock,
        transaction_hash: TxHash,
        finality_status: FinalityStatus,
        receipt: Receipt,
    ) -> Self {
        let receipt = TxReceipt::new(transaction_hash, finality_status, receipt).0;
        Self(TransactionReceiptWithBlockInfo { receipt, block })
    }
}

struct MsgToL1(starknet::core::types::MsgToL1);

impl From<MessageToL1> for MsgToL1 {
    fn from(value: MessageToL1) -> Self {
        MsgToL1(starknet::core::types::MsgToL1 {
            from_address: value.from_address.into(),
            to_address: value.to_address,
            payload: value.payload,
        })
    }
}

struct Event(starknet::core::types::Event);

impl From<katana_primitives::receipt::Event> for Event {
    fn from(value: katana_primitives::receipt::Event) -> Self {
        Event(starknet::core::types::Event {
            from_address: value.from_address.into(),
            keys: value.keys,
            data: value.data,
        })
    }
}

fn to_rpc_resources(resources: receipt::ExecutionResources) -> ExecutionResources {
    match resources.gas {
        receipt::GasUsed::All { l2_gas, l1_gas, l1_data_gas } => {
            ExecutionResources { l2_gas, l1_gas, l1_data_gas }
        }

        // TODO(kariy): update the rpc types to support old format
        receipt::GasUsed::L1Gas { l1_gas } => ExecutionResources {
            l1_gas,
            l2_gas: Default::default(),
            l1_data_gas: Default::default(),
        },
    }
}

fn to_rpc_fee(fee: FeeInfo) -> FeePayment {
    let unit = match fee.unit {
        PriceUnit::Wei => starknet::core::types::PriceUnit::Wei,
        PriceUnit::Fri => starknet::core::types::PriceUnit::Fri,
    };

    FeePayment { amount: fee.overall_fee.into(), unit }
}
