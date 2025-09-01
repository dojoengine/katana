use katana_primitives::block::{BlockHash, BlockNumber, FinalityStatus};
use katana_primitives::fee::{FeeInfo, PriceUnit};
use katana_primitives::receipt::{self, Event, MessageToL1, Receipt};
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;
use serde::{Deserialize, Serialize};
use starknet::core::types::{
    ExecutionResources, ExecutionResult, FeePayment, Hash256, TransactionFinalityStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RpcTxReceipt {
    Invoke(RpcInvokeTxReceipt),
    Deploy(RpcDeployTxReceipt),
    Declare(RpcDeclareTxReceipt),
    L1Handler(RpcL1HandlerTxReceipt),
    DeployAccount(RpcDeployAccountTxReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcInvokeTxReceipt {
    pub transaction_hash: TxHash,
    pub actual_fee: FeePayment,
    pub finality_status: TransactionFinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcL1HandlerTxReceipt {
    pub transaction_hash: TxHash,
    pub message_hash: Hash256,
    pub actual_fee: FeePayment,
    pub finality_status: TransactionFinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeclareTxReceipt {
    pub transaction_hash: TxHash,
    pub actual_fee: FeePayment,
    pub finality_status: TransactionFinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeployTxReceipt {
    pub transaction_hash: TxHash,
    pub actual_fee: FeePayment,
    pub finality_status: TransactionFinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub contract_address: ContractAddress,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeployAccountTxReceipt {
    pub transaction_hash: TxHash,
    pub actual_fee: FeePayment,
    pub finality_status: TransactionFinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub contract_address: ContractAddress,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

impl RpcTxReceipt {
    pub fn new(
        transaction_hash: TxHash,
        finality_status: FinalityStatus,
        receipt: Receipt,
    ) -> Self {
        let finality_status = match finality_status {
            FinalityStatus::AcceptedOnL1 => TransactionFinalityStatus::AcceptedOnL1,
            FinalityStatus::AcceptedOnL2 => TransactionFinalityStatus::AcceptedOnL2,
        };

        match receipt {
            Receipt::Invoke(rct) => {
                let messages_sent = rct.messages_sent;
                let events = rct.events;

                RpcTxReceipt::Invoke(RpcInvokeTxReceipt {
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
                let messages_sent = rct.messages_sent;
                let events = rct.events;

                RpcTxReceipt::Declare(RpcDeclareTxReceipt {
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
                let messages_sent = rct.messages_sent;
                let events = rct.events;

                RpcTxReceipt::L1Handler(RpcL1HandlerTxReceipt {
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
                let messages_sent = rct.messages_sent;
                let events = rct.events;

                RpcTxReceipt::DeployAccount(RpcDeployAccountTxReceipt {
                    events,
                    messages_sent,
                    finality_status,
                    transaction_hash,
                    actual_fee: to_rpc_fee(rct.fee),
                    contract_address: rct.contract_address,
                    execution_resources: to_rpc_resources(rct.execution_resources),
                    execution_result: if let Some(reason) = rct.revert_error {
                        ExecutionResult::Reverted { reason }
                    } else {
                        ExecutionResult::Succeeded
                    },
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxReceiptWithBlockInfo {
    #[serde(flatten)]
    pub receipt: RpcTxReceipt,
    #[serde(flatten)]
    pub block: ReceiptBlockInfo,
}

impl TxReceiptWithBlockInfo {
    pub fn new(
        block: ReceiptBlockInfo,
        transaction_hash: TxHash,
        finality_status: FinalityStatus,
        receipt: Receipt,
    ) -> Self {
        let receipt = RpcTxReceipt::new(transaction_hash, finality_status, receipt);
        Self { receipt, block }
    }
}

/// The block information associated with a receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ReceiptBlockInfo {
    /// The receipt is from a pre-confirmed block.
    PreConfirmed {
        /// Block number.
        block_number: BlockNumber,
    },

    /// The receipt is from a confirmed block.
    Block {
        /// Block hash.
        block_hash: BlockHash,
        /// Block number.
        block_number: BlockNumber,
    },
}

impl<'de> Deserialize<'de> for ReceiptBlockInfo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        #[derive(Debug, Deserialize)]
        struct Raw {
            block_hash: Option<BlockHash>,
            block_number: Option<BlockNumber>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let block_number = raw.block_number.ok_or(Error::custom("`block_number` is missing"))?;

        match raw.block_hash {
            None => Ok(ReceiptBlockInfo::PreConfirmed { block_number }),
            Some(block_hash) => Ok(ReceiptBlockInfo::Block { block_hash, block_number }),
        }
    }
}

fn to_rpc_resources(resources: receipt::ExecutionResources) -> ExecutionResources {
    ExecutionResources {
        l2_gas: resources.gas.l2_gas,
        l1_gas: resources.gas.l1_gas,
        l1_data_gas: resources.gas.l1_data_gas,
    }
}

fn to_rpc_fee(fee: FeeInfo) -> FeePayment {
    let unit = match fee.unit {
        PriceUnit::Wei => starknet::core::types::PriceUnit::Wei,
        PriceUnit::Fri => starknet::core::types::PriceUnit::Fri,
    };

    FeePayment { amount: fee.overall_fee.into(), unit }
}
