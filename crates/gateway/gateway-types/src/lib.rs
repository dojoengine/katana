#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! # Feeder Gateway Types
//!
//! This module defines types that mirror the data structures returned by the Starknet feeder
//! gateway API. Ideally, we would not need to redefine these types, but the feeder gateway requires
//! its own type definitions due to fundamental serialization incompatibilities with the existing
//! types in `katana-primitives` and `katana-rpc-types`. For objects that share the same format, we
//! reuse existing RPC or primitive types whenever possible.
//!
//! ## Affected Types
//!
//! - [`DataAvailabilityMode`]: Integer-based representation
//! - [`ResourceBounds`]: Custom numeric handling
//! - [`ResourceBoundsMapping`]: Uppercase field names, optional `L1_DATA_GAS`
//! - [`InvokeTxV3`]: Uses the custom DA mode and resource bounds
//! - [`DeclareTxV3`]: Uses the custom DA mode and resource bounds
//! - [`DeployAccountTxV1`]: Optional `contract_address` field
//! - [`DeployAccountTxV3`]: Uses the custom DA mode and resource bounds
//! - [`L1HandlerTx`]: Optional `nonce` field

use katana_primitives::block::{BlockHash, BlockNumber};
pub use katana_primitives::class::CasmContractClass;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::{ContractAddress, Felt};
pub use katana_rpc_types::class::RpcSierraContractClass;
use serde::{Deserialize, Serialize};
use starknet::core::types::ResourcePrice;

mod conversion;
mod error;
mod receipt;
mod state_update;
mod transaction;

pub use error::*;
pub use receipt::*;
pub use state_update::*;
pub use transaction::*;

/// The contract class type returns by `/get_class_by_hash` endpoint.
pub type ContractClass = katana_rpc_types::Class;

/// Sequencer public key returned by the `/get_public_key` endpoint.
pub type SequencerPublicKey = Felt;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockId {
    Number(BlockNumber),
    Hash(BlockHash),
    Latest,
    Pending,
}

impl From<BlockNumber> for BlockId {
    fn from(value: BlockNumber) -> Self {
        BlockId::Number(value)
    }
}

impl From<BlockHash> for BlockId {
    fn from(value: BlockHash) -> Self {
        BlockId::Hash(value)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BlockStatus {
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,

    #[serde(rename = "PENDING")]
    Pending,

    #[serde(rename = "ABORTED")]
    Aborted,

    #[serde(rename = "REVERTED")]
    Reverted,

    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,

    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1,
}

/// Block signature returned by the `/signature` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSignature {
    pub block_hash: BlockHash,
    pub signature: [Felt; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreConfirmedBlock {
    pub timestamp: u64,
    pub status: BlockStatus,
    pub sequencer_address: ContractAddress,
    pub l2_gas_price: ResourcePrice,
    pub l1_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    pub starknet_version: String,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub transactions: Vec<ConfirmedTransaction>,
    pub transaction_receipts: Vec<Option<ConfirmedReceipt>>,
    pub transaction_state_diffs: Vec<Option<StateDiff>>,
}

// The reason why we're not using the GasPrices from the `katana_primitives` crate is because
// the serde impl is different. So for now, lets just use starknet-rs types. The type isn't
// that complex anyway so the conversion is simple. But if we can use the primitive types, we
// should.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub status: BlockStatus,
    #[serde(default)]
    pub block_hash: Option<BlockHash>,
    #[serde(default)]
    pub block_number: Option<BlockNumber>,
    pub parent_block_hash: BlockHash,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub timestamp: u64,
    pub sequencer_address: Option<ContractAddress>,
    #[serde(default)]
    pub starknet_version: Option<String>,
    #[serde(default = "default_l2_gas_price")]
    pub l2_gas_price: ResourcePrice,
    pub l1_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    #[serde(default)]
    pub event_commitment: Option<Felt>,
    #[serde(default)]
    pub state_diff_commitment: Option<Felt>,
    #[serde(default)]
    pub state_root: Option<Felt>,
    #[serde(default)]
    pub transaction_commitment: Option<Felt>,
    pub transaction_receipts: Vec<ConfirmedReceipt>,
    pub transactions: Vec<ConfirmedTransaction>,
}

fn default_l2_gas_price() -> ResourcePrice {
    ResourcePrice { price_in_fri: Felt::from(1), price_in_wei: Felt::from(1) }
}
