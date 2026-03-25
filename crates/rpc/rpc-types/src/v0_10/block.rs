//! Block types for Starknet spec v0.10.0.
//!
//! Adds 7 new fields to confirmed block headers:
//! `event_commitment`, `event_count`, `receipt_commitment`, `state_diff_commitment`,
//! `state_diff_length`, `transaction_commitment`, `transaction_count`.
//!
//! These types are constructed via `From` conversions from the shared block types,
//! which carry the commitment data as `#[serde(skip)]` fields.

use katana_primitives::block::{BlockHash, BlockNumber, FinalityStatus};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use starknet::core::types::ResourcePrice;

use crate::transaction::RpcTxWithHash;

// Re-export types that are identical to v0.9.
pub use crate::block::{
    BlockHashAndNumberResponse, BlockNumberResponse, BlockTxCount, PreConfirmedBlockWithReceipts,
    PreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxs, RpcTxWithReceipt,
};

// ---------- BlockWithTxs ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybePreConfirmedBlock {
    Confirmed(BlockWithTxs),
    PreConfirmed(PreConfirmedBlockWithTxs),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockWithTxs {
    pub status: FinalityStatus,
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
    pub event_commitment: Felt,
    pub event_count: u32,
    pub receipt_commitment: Felt,
    pub state_diff_commitment: Felt,
    pub state_diff_length: u32,
    pub transaction_commitment: Felt,
    pub transaction_count: u32,
    pub transactions: Vec<RpcTxWithHash>,
}

impl From<crate::block::BlockWithTxs> for BlockWithTxs {
    fn from(b: crate::block::BlockWithTxs) -> Self {
        Self {
            status: b.status,
            block_hash: b.block_hash,
            parent_hash: b.parent_hash,
            block_number: b.block_number,
            new_root: b.new_root,
            timestamp: b.timestamp,
            sequencer_address: b.sequencer_address,
            l1_gas_price: b.l1_gas_price,
            l2_gas_price: b.l2_gas_price,
            l1_data_gas_price: b.l1_data_gas_price,
            l1_da_mode: b.l1_da_mode,
            starknet_version: b.starknet_version,
            event_commitment: b.event_commitment,
            event_count: b.event_count,
            receipt_commitment: b.receipt_commitment,
            state_diff_commitment: b.state_diff_commitment,
            state_diff_length: b.state_diff_length,
            transaction_commitment: b.transaction_commitment,
            transaction_count: b.transaction_count,
            transactions: b.transactions,
        }
    }
}

impl From<crate::block::MaybePreConfirmedBlock> for MaybePreConfirmedBlock {
    fn from(b: crate::block::MaybePreConfirmedBlock) -> Self {
        match b {
            crate::block::MaybePreConfirmedBlock::Confirmed(b) => {
                MaybePreConfirmedBlock::Confirmed(b.into())
            }
            crate::block::MaybePreConfirmedBlock::PreConfirmed(b) => {
                MaybePreConfirmedBlock::PreConfirmed(b)
            }
        }
    }
}

// ---------- BlockWithTxHashes ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GetBlockWithTxHashesResponse {
    Block(BlockWithTxHashes),
    PreConfirmed(PreConfirmedBlockWithTxHashes),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockWithTxHashes {
    pub status: FinalityStatus,
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
    pub event_commitment: Felt,
    pub event_count: u32,
    pub receipt_commitment: Felt,
    pub state_diff_commitment: Felt,
    pub state_diff_length: u32,
    pub transaction_commitment: Felt,
    pub transaction_count: u32,
    pub transactions: Vec<TxHash>,
}

impl From<crate::block::BlockWithTxHashes> for BlockWithTxHashes {
    fn from(b: crate::block::BlockWithTxHashes) -> Self {
        Self {
            status: b.status,
            block_hash: b.block_hash,
            parent_hash: b.parent_hash,
            block_number: b.block_number,
            new_root: b.new_root,
            timestamp: b.timestamp,
            sequencer_address: b.sequencer_address,
            l1_gas_price: b.l1_gas_price,
            l2_gas_price: b.l2_gas_price,
            l1_data_gas_price: b.l1_data_gas_price,
            l1_da_mode: b.l1_da_mode,
            starknet_version: b.starknet_version,
            event_commitment: b.event_commitment,
            event_count: b.event_count,
            receipt_commitment: b.receipt_commitment,
            state_diff_commitment: b.state_diff_commitment,
            state_diff_length: b.state_diff_length,
            transaction_commitment: b.transaction_commitment,
            transaction_count: b.transaction_count,
            transactions: b.transactions,
        }
    }
}

impl From<crate::block::GetBlockWithTxHashesResponse> for GetBlockWithTxHashesResponse {
    fn from(r: crate::block::GetBlockWithTxHashesResponse) -> Self {
        match r {
            crate::block::GetBlockWithTxHashesResponse::Block(b) => {
                GetBlockWithTxHashesResponse::Block(b.into())
            }
            crate::block::GetBlockWithTxHashesResponse::PreConfirmed(b) => {
                GetBlockWithTxHashesResponse::PreConfirmed(b)
            }
        }
    }
}

// ---------- BlockWithReceipts ----------

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum GetBlockWithReceiptsResponse {
    Block(BlockWithReceipts),
    PreConfirmed(PreConfirmedBlockWithReceipts),
}

impl<'de> Deserialize<'de> for GetBlockWithReceiptsResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value =
            serde_json::Value::deserialize(deserializer).map_err(serde::de::Error::custom)?;

        if value.get("block_hash").is_some() {
            let block = serde_json::from_value::<BlockWithReceipts>(value)
                .map_err(serde::de::Error::custom)?;
            Ok(GetBlockWithReceiptsResponse::Block(block))
        } else {
            let block = serde_json::from_value::<PreConfirmedBlockWithReceipts>(value)
                .map_err(serde::de::Error::custom)?;
            Ok(GetBlockWithReceiptsResponse::PreConfirmed(block))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockWithReceipts {
    pub status: FinalityStatus,
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
    pub event_commitment: Felt,
    pub event_count: u32,
    pub receipt_commitment: Felt,
    pub state_diff_commitment: Felt,
    pub state_diff_length: u32,
    pub transaction_commitment: Felt,
    pub transaction_count: u32,
    pub transactions: Vec<RpcTxWithReceipt>,
}

impl From<crate::block::BlockWithReceipts> for BlockWithReceipts {
    fn from(b: crate::block::BlockWithReceipts) -> Self {
        Self {
            status: b.status,
            block_hash: b.block_hash,
            parent_hash: b.parent_hash,
            block_number: b.block_number,
            new_root: b.new_root,
            timestamp: b.timestamp,
            sequencer_address: b.sequencer_address,
            l1_gas_price: b.l1_gas_price,
            l2_gas_price: b.l2_gas_price,
            l1_data_gas_price: b.l1_data_gas_price,
            l1_da_mode: b.l1_da_mode,
            starknet_version: b.starknet_version,
            event_commitment: b.event_commitment,
            event_count: b.event_count,
            receipt_commitment: b.receipt_commitment,
            state_diff_commitment: b.state_diff_commitment,
            state_diff_length: b.state_diff_length,
            transaction_commitment: b.transaction_commitment,
            transaction_count: b.transaction_count,
            transactions: b.transactions,
        }
    }
}

impl From<crate::block::GetBlockWithReceiptsResponse> for GetBlockWithReceiptsResponse {
    fn from(r: crate::block::GetBlockWithReceiptsResponse) -> Self {
        match r {
            crate::block::GetBlockWithReceiptsResponse::Block(b) => {
                GetBlockWithReceiptsResponse::Block(b.into())
            }
            crate::block::GetBlockWithReceiptsResponse::PreConfirmed(b) => {
                GetBlockWithReceiptsResponse::PreConfirmed(b)
            }
        }
    }
}
