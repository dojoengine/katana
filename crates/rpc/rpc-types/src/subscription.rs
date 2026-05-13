use katana_primitives::block::{BlockHash, BlockNumber, FinalityStatus};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use starknet::core::types::ResourcePrice;

use crate::event::EmittedEvent;
use crate::receipt::TxReceiptWithBlockInfo;
use crate::transaction::RpcTxWithHash;

/// Block header notification for `starknet_subscriptionNewHeads`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionBlockHeader {
    pub block_hash: BlockHash,
    pub parent_hash: BlockHash,
    pub block_number: BlockNumber,
    pub new_root: Felt,
    pub timestamp: u64,
    pub sequencer_address: ContractAddress,
    pub l1_gas_price: ResourcePrice,
    pub l2_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub starknet_version: String,
}

impl SubscriptionBlockHeader {
    /// Create a new block header notification from a primitive header and block hash.
    pub fn new(block_hash: BlockHash, header: katana_primitives::block::Header) -> Self {
        Self {
            block_hash,
            parent_hash: header.parent_hash,
            block_number: header.number,
            new_root: header.state_root,
            timestamp: header.timestamp,
            sequencer_address: header.sequencer_address,
            l1_gas_price: ResourcePrice {
                price_in_wei: header.l1_gas_prices.eth.get().into(),
                price_in_fri: header.l1_gas_prices.strk.get().into(),
            },
            l2_gas_price: ResourcePrice {
                price_in_wei: header.l2_gas_prices.eth.get().into(),
                price_in_fri: header.l2_gas_prices.strk.get().into(),
            },
            l1_data_gas_price: ResourcePrice {
                price_in_wei: header.l1_data_gas_prices.eth.get().into(),
                price_in_fri: header.l1_data_gas_prices.strk.get().into(),
            },
            l1_da_mode: header.l1_da_mode,
            starknet_version: header.starknet_version.to_string(),
        }
    }
}

/// Reorg notification data.
///
/// Sent when a chain reorganization is detected. Contains the range of orphaned blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorgData {
    pub starting_block_hash: BlockHash,
    pub starting_block_number: BlockNumber,
    pub ending_block_hash: BlockHash,
    pub ending_block_number: BlockNumber,
}

/// Transaction status update notification for `starknet_subscriptionTransactionStatus`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionStatusUpdate {
    pub transaction_hash: TxHash,
    #[serde(flatten)]
    pub status: NewTxnStatus,
}

/// The status result within a transaction status update notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTxnStatus {
    pub finality_status: FinalityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<ExecutionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

/// Execution status of a transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionStatus {
    #[serde(rename = "SUCCEEDED")]
    Succeeded,
    #[serde(rename = "REVERTED")]
    Reverted,
}

/// Emitted event with finality status for `starknet_subscriptionEvents`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmittedEventWithFinalityStatus {
    #[serde(flatten)]
    pub event: EmittedEvent,
    pub finality_status: FinalityStatus,
}

/// Transaction with finality status for `starknet_subscriptionNewTransaction`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxWithFinalityStatus {
    #[serde(flatten)]
    pub transaction: RpcTxWithHash,
    pub finality_status: FinalityStatus,
}

/// Receipt notification for `starknet_subscriptionNewTransactionReceipts`.
pub type NewTransactionReceipt = TxReceiptWithBlockInfo;
