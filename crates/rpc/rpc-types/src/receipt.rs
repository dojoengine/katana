use katana_primitives::block::{BlockHash, BlockNumber, FinalityStatus};
use katana_primitives::fee::{FeeInfo, PriceUnit};
use katana_primitives::receipt::{self, Event, MessageToL1, Receipt};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use starknet::core::types::Hash256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcTxReceiptWithHash {
    /// The hash of the transaction associated with the receipt.
    pub transaction_hash: TxHash,
    #[serde(flatten)]
    pub receipt: RpcTxReceipt,
}

impl RpcTxReceiptWithHash {
    pub fn new(
        transaction_hash: TxHash,
        receipt: Receipt,
        finality_status: FinalityStatus,
    ) -> Self {
        Self { transaction_hash, receipt: RpcTxReceipt::new(receipt, finality_status) }
    }
}

/// Fee payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeePayment {
    /// Amount paid
    pub amount: Felt,
    /// Units in which the fee is given
    pub unit: PriceUnit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RpcTxReceipt {
    #[serde(rename = "INVOKE")]
    Invoke(RpcInvokeTxReceipt),

    #[serde(rename = "DEPLOY")]
    Deploy(RpcDeployTxReceipt),

    #[serde(rename = "DECLARE")]
    Declare(RpcDeclareTxReceipt),

    #[serde(rename = "L1_HANDLER")]
    L1Handler(RpcL1HandlerTxReceipt),

    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(RpcDeployAccountTxReceipt),
}

impl RpcTxReceipt {
    pub fn execution_resources(&self) -> &ExecutionResources {
        match self {
            RpcTxReceipt::Invoke(receipt) => &receipt.execution_resources,
            RpcTxReceipt::Deploy(receipt) => &receipt.execution_resources,
            RpcTxReceipt::Declare(receipt) => &receipt.execution_resources,
            RpcTxReceipt::L1Handler(receipt) => &receipt.execution_resources,
            RpcTxReceipt::DeployAccount(receipt) => &receipt.execution_resources,
        }
    }

    pub fn execution_result(&self) -> &ExecutionResult {
        match self {
            RpcTxReceipt::Invoke(receipt) => &receipt.execution_result,
            RpcTxReceipt::Deploy(receipt) => &receipt.execution_result,
            RpcTxReceipt::Declare(receipt) => &receipt.execution_result,
            RpcTxReceipt::L1Handler(receipt) => &receipt.execution_result,
            RpcTxReceipt::DeployAccount(receipt) => &receipt.execution_result,
        }
    }

    pub fn finality_status(&self) -> &FinalityStatus {
        match self {
            RpcTxReceipt::Invoke(receipt) => &receipt.finality_status,
            RpcTxReceipt::Deploy(receipt) => &receipt.finality_status,
            RpcTxReceipt::Declare(receipt) => &receipt.finality_status,
            RpcTxReceipt::L1Handler(receipt) => &receipt.finality_status,
            RpcTxReceipt::DeployAccount(receipt) => &receipt.finality_status,
        }
    }

    pub fn actual_fee(&self) -> &FeePayment {
        match self {
            RpcTxReceipt::Invoke(receipt) => &receipt.actual_fee,
            RpcTxReceipt::Deploy(receipt) => &receipt.actual_fee,
            RpcTxReceipt::Declare(receipt) => &receipt.actual_fee,
            RpcTxReceipt::L1Handler(receipt) => &receipt.actual_fee,
            RpcTxReceipt::DeployAccount(receipt) => &receipt.actual_fee,
        }
    }

    pub fn events(&self) -> &[Event] {
        match self {
            RpcTxReceipt::Invoke(receipt) => &receipt.events,
            RpcTxReceipt::Deploy(receipt) => &receipt.events,
            RpcTxReceipt::Declare(receipt) => &receipt.events,
            RpcTxReceipt::L1Handler(receipt) => &receipt.events,
            RpcTxReceipt::DeployAccount(receipt) => &receipt.events,
        }
    }

    pub fn messages_sent(&self) -> &[MessageToL1] {
        match self {
            RpcTxReceipt::Invoke(receipt) => &receipt.messages_sent,
            RpcTxReceipt::Deploy(receipt) => &receipt.messages_sent,
            RpcTxReceipt::Declare(receipt) => &receipt.messages_sent,
            RpcTxReceipt::L1Handler(receipt) => &receipt.messages_sent,
            RpcTxReceipt::DeployAccount(receipt) => &receipt.messages_sent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcInvokeTxReceipt {
    pub actual_fee: FeePayment,
    pub finality_status: FinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcL1HandlerTxReceipt {
    pub message_hash: Hash256,
    pub actual_fee: FeePayment,
    pub finality_status: FinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeclareTxReceipt {
    pub actual_fee: FeePayment,
    pub finality_status: FinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeployTxReceipt {
    pub actual_fee: FeePayment,
    pub finality_status: FinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub contract_address: ContractAddress,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeployAccountTxReceipt {
    pub actual_fee: FeePayment,
    pub finality_status: FinalityStatus,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub contract_address: ContractAddress,
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

impl RpcTxReceipt {
    fn new(receipt: Receipt, finality_status: FinalityStatus) -> Self {
        match receipt {
            Receipt::Invoke(rct) => {
                let messages_sent = rct.messages_sent;
                let events = rct.events;

                RpcTxReceipt::Invoke(RpcInvokeTxReceipt {
                    events,
                    messages_sent,
                    finality_status,
                    actual_fee: rct.fee.into(),
                    execution_resources: rct.execution_resources.into(),
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
                    actual_fee: rct.fee.into(),
                    execution_resources: rct.execution_resources.into(),
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
                    actual_fee: rct.fee.into(),
                    execution_resources: rct.execution_resources.into(),
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
                    actual_fee: rct.fee.into(),
                    contract_address: rct.contract_address,
                    execution_resources: rct.execution_resources.into(),
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
    /// The hash of the transaction associated with the receipt.
    pub transaction_hash: TxHash,
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
        let receipt = RpcTxReceipt::new(receipt, finality_status);
        Self { transaction_hash, receipt, block }
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

impl ReceiptBlockInfo {
    /// Returns the block number of this block info.
    ///
    /// This is the block number of the block that contains the transaction.
    pub fn block_number(&self) -> BlockNumber {
        match self {
            ReceiptBlockInfo::PreConfirmed { block_number } => *block_number,
            ReceiptBlockInfo::Block { block_number, .. } => *block_number,
        }
    }

    /// Returns the block hash if the receipt is from a confirmed block. Otherwise, returns `None`.
    pub fn block_hash(&self) -> Option<BlockHash> {
        match self {
            ReceiptBlockInfo::PreConfirmed { .. } => None,
            ReceiptBlockInfo::Block { block_hash, .. } => Some(*block_hash),
        }
    }
}

impl<'de> Deserialize<'de> for ReceiptBlockInfo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Debug, Deserialize)]
        struct Json {
            block_number: BlockNumber,
            block_hash: Option<BlockHash>,
        }

        let raw = Json::deserialize(deserializer)?;
        let block_number = raw.block_number;
        let block_hash = raw.block_hash;

        match block_hash {
            None => Ok(ReceiptBlockInfo::PreConfirmed { block_number }),
            Some(block_hash) => Ok(ReceiptBlockInfo::Block { block_hash, block_number }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "execution_status")]
pub enum ExecutionResult {
    #[serde(rename = "SUCCEEDED")]
    Succeeded,

    #[serde(rename = "REVERTED")]
    Reverted {
        #[serde(rename = "revert_reason")]
        reason: String,
    },
}

/// The resources consumed by the transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionResources {
    /// L1 gas consumed by this transaction, used for L2-->L1 messages and state updates if blobs
    /// are not used
    pub l1_gas: u64,
    /// Data gas consumed by this transaction, 0 if blobs are not used
    pub l1_data_gas: u64,
    /// L2 gas consumed by this transaction, used for computation and calldata
    pub l2_gas: u64,
}

impl From<receipt::ExecutionResources> for ExecutionResources {
    fn from(resources: receipt::ExecutionResources) -> Self {
        ExecutionResources {
            l2_gas: resources.gas.l2_gas,
            l1_gas: resources.gas.l1_gas,
            l1_data_gas: resources.gas.l1_data_gas,
        }
    }
}

impl From<FeeInfo> for FeePayment {
    fn from(fee: FeeInfo) -> Self {
        FeePayment { amount: fee.overall_fee.into(), unit: fee.unit }
    }
}
