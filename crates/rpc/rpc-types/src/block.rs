use katana_primitives::block::{
    Block, BlockHash, BlockNumber, FinalityStatus, Header, PartialHeader,
};
use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::{TxHash, TxWithHash};
use serde::{Deserialize, Serialize};
use starknet::core::types::{
    BlockStatus, L1DataAvailabilityMode, ResourcePrice, TransactionWithReceipt,
};

use crate::receipt::TxReceipt;
use crate::transaction::TxContent;

pub type BlockTxCount = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockWithTxs(starknet::core::types::BlockWithTxs);

impl BlockWithTxs {
    pub fn new(block_hash: BlockHash, block: Block, finality_status: FinalityStatus) -> Self {
        let l1_gas_price = ResourcePrice {
            price_in_wei: block.header.l1_gas_prices.eth.get().into(),
            price_in_fri: block.header.l1_gas_prices.strk.get().into(),
        };

        let l2_gas_price = ResourcePrice {
            price_in_wei: block.header.l2_gas_prices.eth.get().into(),
            price_in_fri: block.header.l2_gas_prices.strk.get().into(),
        };

        let l1_data_gas_price = ResourcePrice {
            price_in_wei: block.header.l1_data_gas_prices.eth.get().into(),
            price_in_fri: block.header.l1_data_gas_prices.strk.get().into(),
        };

        let transactions =
            block.body.into_iter().map(|tx| crate::transaction::Tx::from(tx).0).collect();

        Self(starknet::core::types::BlockWithTxs {
            block_hash,
            l1_gas_price,
            l2_gas_price,
            transactions,
            new_root: block.header.state_root,
            timestamp: block.header.timestamp,
            block_number: block.header.number,
            parent_hash: block.header.parent_hash,
            starknet_version: block.header.starknet_version.to_string(),
            sequencer_address: block.header.sequencer_address.into(),
            status: match finality_status {
                FinalityStatus::AcceptedOnL1 => BlockStatus::AcceptedOnL1,
                FinalityStatus::AcceptedOnL2 => BlockStatus::AcceptedOnL2,
            },
            l1_da_mode: match block.header.l1_da_mode {
                katana_primitives::da::L1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
                katana_primitives::da::L1DataAvailabilityMode::Calldata => {
                    L1DataAvailabilityMode::Calldata
                }
            },
            l1_data_gas_price,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PendingBlockWithTxs(starknet::core::types::PendingBlockWithTxs);

impl PendingBlockWithTxs {
    pub fn new(header: PartialHeader, transactions: Vec<TxWithHash>) -> Self {
        let transactions =
            transactions.into_iter().map(|tx| crate::transaction::Tx::from(tx).0).collect();

        let l1_gas_price = ResourcePrice {
            price_in_wei: header.l1_gas_prices.eth.get().into(),
            price_in_fri: header.l1_gas_prices.strk.get().into(),
        };

        let l2_gas_price = ResourcePrice {
            price_in_wei: header.l2_gas_prices.eth.get().into(),
            price_in_fri: header.l2_gas_prices.strk.get().into(),
        };

        let l1_data_gas_price =
            ResourcePrice { price_in_fri: Default::default(), price_in_wei: Default::default() };

        Self(starknet::core::types::PendingBlockWithTxs {
            transactions,
            l1_gas_price,
            l2_gas_price,
            timestamp: header.timestamp,
            parent_hash: header.parent_hash,
            starknet_version: header.starknet_version.to_string(),
            sequencer_address: header.sequencer_address.into(),
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            l1_data_gas_price,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybePendingBlockWithTxs {
    Pending(PendingBlockWithTxs),
    Block(BlockWithTxs),
}

impl From<starknet::core::types::MaybePendingBlockWithTxs> for MaybePendingBlockWithTxs {
    fn from(value: starknet::core::types::MaybePendingBlockWithTxs) -> Self {
        match value {
            starknet::core::types::MaybePendingBlockWithTxs::PendingBlock(block) => {
                MaybePendingBlockWithTxs::Pending(PendingBlockWithTxs(block))
            }
            starknet::core::types::MaybePendingBlockWithTxs::Block(block) => {
                MaybePendingBlockWithTxs::Block(BlockWithTxs(block))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockWithTxHashes(starknet::core::types::BlockWithTxHashes);

impl BlockWithTxHashes {
    pub fn new(
        block_hash: BlockHash,
        block: katana_primitives::block::BlockWithTxHashes,
        finality_status: FinalityStatus,
    ) -> Self {
        let l1_gas_price = ResourcePrice {
            price_in_wei: block.header.l1_gas_prices.eth.get().into(),
            price_in_fri: block.header.l1_gas_prices.strk.get().into(),
        };

        let l2_gas_price = ResourcePrice {
            price_in_wei: block.header.l2_gas_prices.eth.get().into(),
            price_in_fri: block.header.l2_gas_prices.strk.get().into(),
        };

        let l1_data_gas_price = ResourcePrice {
            price_in_wei: block.header.l1_data_gas_prices.eth.get().into(),
            price_in_fri: block.header.l1_data_gas_prices.strk.get().into(),
        };

        Self(starknet::core::types::BlockWithTxHashes {
            block_hash,
            l1_gas_price,
            l2_gas_price,
            transactions: block.body,
            new_root: block.header.state_root,
            timestamp: block.header.timestamp,
            block_number: block.header.number,
            parent_hash: block.header.parent_hash,
            starknet_version: block.header.starknet_version.to_string(),
            sequencer_address: block.header.sequencer_address.into(),
            status: match finality_status {
                FinalityStatus::AcceptedOnL1 => BlockStatus::AcceptedOnL1,
                FinalityStatus::AcceptedOnL2 => BlockStatus::AcceptedOnL2,
            },
            l1_da_mode: match block.header.l1_da_mode {
                katana_primitives::da::L1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
                katana_primitives::da::L1DataAvailabilityMode::Calldata => {
                    L1DataAvailabilityMode::Calldata
                }
            },
            l1_data_gas_price,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PendingBlockWithTxHashes(starknet::core::types::PendingBlockWithTxHashes);

impl PendingBlockWithTxHashes {
    pub fn new(header: PartialHeader, transactions: Vec<TxHash>) -> Self {
        let l1_gas_price = ResourcePrice {
            price_in_wei: header.l1_gas_prices.eth.get().into(),
            price_in_fri: header.l1_gas_prices.strk.get().into(),
        };

        let l2_gas_price = ResourcePrice {
            price_in_wei: header.l2_gas_prices.eth.get().into(),
            price_in_fri: header.l2_gas_prices.strk.get().into(),
        };

        let l1_data_gas_price = ResourcePrice {
            price_in_wei: header.l1_data_gas_prices.eth.get().into(),
            price_in_fri: header.l1_data_gas_prices.strk.get().into(),
        };

        Self(starknet::core::types::PendingBlockWithTxHashes {
            transactions,
            l1_gas_price,
            l2_gas_price,
            timestamp: header.timestamp,
            parent_hash: header.parent_hash,
            starknet_version: header.starknet_version.to_string(),
            sequencer_address: header.sequencer_address.into(),
            l1_da_mode: match header.l1_da_mode {
                katana_primitives::da::L1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
                katana_primitives::da::L1DataAvailabilityMode::Calldata => {
                    L1DataAvailabilityMode::Calldata
                }
            },
            l1_data_gas_price,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybePendingBlockWithTxHashes {
    Pending(PendingBlockWithTxHashes),
    Block(BlockWithTxHashes),
}

impl From<starknet::core::types::MaybePendingBlockWithTxHashes> for MaybePendingBlockWithTxHashes {
    fn from(value: starknet::core::types::MaybePendingBlockWithTxHashes) -> Self {
        match value {
            starknet::core::types::MaybePendingBlockWithTxHashes::PendingBlock(block) => {
                MaybePendingBlockWithTxHashes::Pending(PendingBlockWithTxHashes(block))
            }
            starknet::core::types::MaybePendingBlockWithTxHashes::Block(block) => {
                MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes(block))
            }
        }
    }
}

/// The response object for the `starknet_blockHashAndNumber` method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockHashAndNumber {
    /// The block's hash.
    pub block_hash: BlockHash,
    /// The block's number (height).
    pub block_number: u64,
}

impl BlockHashAndNumber {
    pub fn new(block_hash: BlockHash, block_number: BlockNumber) -> Self {
        Self { block_hash, block_number }
    }
}

impl From<(BlockHash, BlockNumber)> for BlockHashAndNumber {
    fn from((hash, number): (BlockHash, BlockNumber)) -> Self {
        Self::new(hash, number)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockWithReceipts(starknet::core::types::BlockWithReceipts);

impl BlockWithReceipts {
    pub fn new(
        hash: BlockHash,
        header: Header,
        finality_status: FinalityStatus,
        receipts: impl Iterator<Item = (TxWithHash, Receipt)>,
    ) -> Self {
        let l1_gas_price = ResourcePrice {
            price_in_wei: header.l1_gas_prices.eth.get().into(),
            price_in_fri: header.l1_gas_prices.strk.get().into(),
        };

        let l2_gas_price = ResourcePrice {
            price_in_wei: header.l2_gas_prices.eth.get().into(),
            price_in_fri: header.l2_gas_prices.strk.get().into(),
        };

        let l1_data_gas_price = ResourcePrice {
            price_in_wei: header.l1_data_gas_prices.eth.get().into(),
            price_in_fri: header.l1_data_gas_prices.strk.get().into(),
        };

        let transactions = receipts
            .map(|(tx_with_hash, receipt)| {
                let receipt = TxReceipt::new(tx_with_hash.hash, finality_status, receipt).0;
                let transaction = TxContent::from(tx_with_hash).0;
                TransactionWithReceipt { transaction, receipt }
            })
            .collect();

        Self(starknet::core::types::BlockWithReceipts {
            status: match finality_status {
                FinalityStatus::AcceptedOnL1 => BlockStatus::AcceptedOnL1,
                FinalityStatus::AcceptedOnL2 => BlockStatus::AcceptedOnL2,
            },
            block_hash: hash,
            parent_hash: header.parent_hash,
            block_number: header.number,
            new_root: header.state_root,
            timestamp: header.timestamp,
            sequencer_address: header.sequencer_address.into(),
            l1_gas_price,
            l2_gas_price,
            l1_data_gas_price,
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: header.starknet_version.to_string(),
            transactions,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PendingBlockWithReceipts(starknet::core::types::PendingBlockWithReceipts);

impl PendingBlockWithReceipts {
    pub fn new(
        header: PartialHeader,
        receipts: impl Iterator<Item = (TxWithHash, Receipt)>,
    ) -> Self {
        let l1_gas_price = ResourcePrice {
            price_in_wei: header.l1_gas_prices.eth.get().into(),
            price_in_fri: header.l1_gas_prices.strk.get().into(),
        };

        let l2_gas_price = ResourcePrice {
            price_in_wei: header.l2_gas_prices.eth.get().into(),
            price_in_fri: header.l2_gas_prices.strk.get().into(),
        };

        let l1_data_gas_price = ResourcePrice {
            price_in_wei: header.l1_data_gas_prices.eth.get().into(),
            price_in_fri: header.l1_data_gas_prices.strk.get().into(),
        };

        let transactions = receipts
            .map(|(tx_with_hash, receipt)| {
                let receipt =
                    TxReceipt::new(tx_with_hash.hash, FinalityStatus::AcceptedOnL2, receipt).0;
                let transaction = TxContent::from(tx_with_hash).0;
                TransactionWithReceipt { transaction, receipt }
            })
            .collect();

        Self(starknet::core::types::PendingBlockWithReceipts {
            transactions,
            l1_gas_price,
            l2_gas_price,
            timestamp: header.timestamp,
            sequencer_address: header.sequencer_address.into(),
            parent_hash: header.parent_hash,
            l1_da_mode: match header.l1_da_mode {
                katana_primitives::da::L1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
                katana_primitives::da::L1DataAvailabilityMode::Calldata => {
                    L1DataAvailabilityMode::Calldata
                }
            },
            l1_data_gas_price,
            starknet_version: header.starknet_version.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybePendingBlockWithReceipts {
    Pending(PendingBlockWithReceipts),
    Block(BlockWithReceipts),
}

impl From<starknet::core::types::MaybePendingBlockWithReceipts> for MaybePendingBlockWithReceipts {
    fn from(value: starknet::core::types::MaybePendingBlockWithReceipts) -> Self {
        match value {
            starknet::core::types::MaybePendingBlockWithReceipts::PendingBlock(block) => {
                MaybePendingBlockWithReceipts::Pending(PendingBlockWithReceipts(block))
            }
            starknet::core::types::MaybePendingBlockWithReceipts::Block(block) => {
                MaybePendingBlockWithReceipts::Block(BlockWithReceipts(block))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::felt;
    use serde_json::{json, Value};

    use super::BlockHashAndNumber;

    #[rstest::rstest]
    #[case(json!({
		"block_hash": "0x69ff022845ab47276b5b2c30d17e19b3a87192228e1495ec332180f52e9850e",
		"block_number": 1660537
    }), BlockHashAndNumber {
	    block_hash: felt!("0x69ff022845ab47276b5b2c30d17e19b3a87192228e1495ec332180f52e9850e"),
	    block_number: 1660537
    })]
    #[case(json!({
		"block_hash": "0x0",
		"block_number": 0
    }), BlockHashAndNumber {
	    block_hash: felt!("0x0"),
	    block_number: 0
    })]
    fn block_hash_and_number(#[case] json: Value, #[case] expected: BlockHashAndNumber) {
        let deserialized = serde_json::from_value::<BlockHashAndNumber>(json.clone()).unwrap();
        similar_asserts::assert_eq!(deserialized, expected);
        let serialized = serde_json::to_value(deserialized).unwrap();
        similar_asserts::assert_eq!(serialized, json);
    }
}
