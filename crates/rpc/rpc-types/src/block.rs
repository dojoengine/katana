use katana_primitives::block::{
    Block, BlockHash, BlockNumber, FinalityStatus, Header, PartialHeader,
};
use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::{TxHash, TxWithHash};
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use starknet::core::types::{
    BlockStatus, L1DataAvailabilityMode, ResourcePrice, TransactionContent,
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
pub struct PreConfirmedBlockWithTxs(starknet::core::types::PreConfirmedBlockWithTxs);

impl PreConfirmedBlockWithTxs {
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

        Self(starknet::core::types::PreConfirmedBlockWithTxs {
            transactions,
            l1_gas_price,
            l2_gas_price,
            timestamp: header.timestamp,
            block_number: header.number,
            starknet_version: header.starknet_version.to_string(),
            sequencer_address: header.sequencer_address.into(),
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            l1_data_gas_price,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybePreConfirmedBlockWithTxs {
    PreConfirmed(PreConfirmedBlockWithTxs),
    Block(BlockWithTxs),
}

impl From<starknet::core::types::MaybePreConfirmedBlockWithTxs> for MaybePreConfirmedBlockWithTxs {
    fn from(value: starknet::core::types::MaybePreConfirmedBlockWithTxs) -> Self {
        match value {
            starknet::core::types::MaybePreConfirmedBlockWithTxs::PreConfirmedBlock(block) => {
                MaybePreConfirmedBlockWithTxs::PreConfirmed(PreConfirmedBlockWithTxs(block))
            }
            starknet::core::types::MaybePreConfirmedBlockWithTxs::Block(block) => {
                MaybePreConfirmedBlockWithTxs::Block(BlockWithTxs(block))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockWithTxHashes(pub starknet::core::types::BlockWithTxHashes);

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
pub struct PreConfirmedBlockWithTxHashes(starknet::core::types::PreConfirmedBlockWithTxHashes);

impl PreConfirmedBlockWithTxHashes {
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

        Self(starknet::core::types::PreConfirmedBlockWithTxHashes {
            transactions,
            l1_gas_price,
            l2_gas_price,
            timestamp: header.timestamp,
            block_number: header.number,
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
pub enum MaybePreConfirmedBlockWithTxHashes {
    PreConfirmed(PreConfirmedBlockWithTxHashes),
    Block(BlockWithTxHashes),
}

impl From<starknet::core::types::MaybePreConfirmedBlockWithTxHashes>
    for MaybePreConfirmedBlockWithTxHashes
{
    fn from(value: starknet::core::types::MaybePreConfirmedBlockWithTxHashes) -> Self {
        match value {
            starknet::core::types::MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(block) => {
                MaybePreConfirmedBlockWithTxHashes::PreConfirmed(PreConfirmedBlockWithTxHashes(
                    block,
                ))
            }
            starknet::core::types::MaybePreConfirmedBlockWithTxHashes::Block(block) => {
                MaybePreConfirmedBlockWithTxHashes::Block(BlockWithTxHashes(block))
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
#[serde(untagged)]
pub enum MaybePreConfirmedBlockWithReceipts {
    PreConfirmed(PreConfirmedBlockWithReceipts),
    Block(BlockWithReceipts),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockWithReceipts {
    pub status: BlockStatus,
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
    pub transactions: Vec<TxWithReceipt>,
}

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
                let receipt = TxReceipt::new(tx_with_hash.hash, finality_status, receipt);
                let transaction = TxContent::from(tx_with_hash).0;
                TxWithReceipt { transaction, receipt }
            })
            .collect();

        Self {
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxWithReceipt {
    transaction: TransactionContent,
    receipt: TxReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreConfirmedBlockWithReceipts {
    pub transactions: Vec<TxWithReceipt>,
    pub block_number: BlockNumber,
    pub timestamp: u64,
    pub sequencer_address: ContractAddress,
    pub l1_gas_price: ResourcePrice,
    pub l2_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub starknet_version: String,
}

impl PreConfirmedBlockWithReceipts {
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
                    TxReceipt::new(tx_with_hash.hash, FinalityStatus::AcceptedOnL2, receipt);
                let transaction = TxContent::from(tx_with_hash).0;
                TxWithReceipt { transaction, receipt }
            })
            .collect();

        Self {
            transactions,
            l1_gas_price,
            l2_gas_price,
            timestamp: header.timestamp,
            sequencer_address: header.sequencer_address.into(),
            block_number: header.number,
            l1_da_mode: match header.l1_da_mode {
                katana_primitives::da::L1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
                katana_primitives::da::L1DataAvailabilityMode::Calldata => {
                    L1DataAvailabilityMode::Calldata
                }
            },
            l1_data_gas_price,
            starknet_version: header.starknet_version.to_string(),
        }
    }
}

impl From<PreConfirmedBlockWithReceipts> for MaybePreConfirmedBlockWithReceipts {
    fn from(value: PreConfirmedBlockWithReceipts) -> Self {
        MaybePreConfirmedBlockWithReceipts::PreConfirmed(value)
    }
}

impl From<BlockWithReceipts> for MaybePreConfirmedBlockWithReceipts {
    fn from(value: BlockWithReceipts) -> Self {
        MaybePreConfirmedBlockWithReceipts::Block(value)
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
