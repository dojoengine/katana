use std::collections::{BTreeMap, BTreeSet};

use katana_primitives::block::{BlockHash, BlockNumber};
pub use katana_primitives::class::CasmContractClass;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{Nonce, StorageKey, StorageValue};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::version::StarknetVersion;
use katana_primitives::{ContractAddress, Felt};
pub use katana_rpc_types::class::SierraClass;
use serde::{Deserialize, Serialize};
use starknet::core::types::ResourcePrice;

mod receipt;
mod transaction;

pub use receipt::*;
pub use transaction::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockId {
    Number(BlockNumber),
    Hash(BlockHash),
    Latest,
}

/// The contract class type returns by `/get_class_by_hash` endpoint.
pub type ContractClass = katana_rpc_types::Class;

/// The state update type returns by `/get_state_update` endpoint.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateUpdate {
    pub block_hash: Option<BlockHash>,
    pub new_root: Option<Felt>,
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDiff {
    pub storage_diffs: BTreeMap<ContractAddress, Vec<StorageDiff>>,
    pub deployed_contracts: Vec<DeployedContract>,
    pub old_declared_contracts: Vec<Felt>,
    pub declared_classes: Vec<DeclaredContract>,
    pub nonces: BTreeMap<ContractAddress, Nonce>,
    pub replaced_classes: Vec<DeployedContract>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDiff {
    pub key: StorageKey,
    pub value: StorageValue,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployedContract {
    pub address: ContractAddress,
    pub class_hash: ClassHash,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredContract {
    pub class_hash: ClassHash,
    pub compiled_class_hash: CompiledClassHash,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BlockStatus {
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

/// The state update type returns by `/get_state_update` endpoint, with `includeBlock=true`.
#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct StateUpdateWithBlock {
    pub state_update: StateUpdate,
    pub block: Block,
}

// The reason why we're not using the GasPrices from the `katana_primitives` crate is because
// the serde impl is different. So for now, lets just use starknet-rs types. The type isn't
// that complex anyway so the conversion is simple. But if we can use the primitive types, we
// should.
#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct Block {
    #[serde(default)]
    pub block_hash: Option<BlockHash>,
    #[serde(default)]
    pub block_number: Option<BlockNumber>,
    pub parent_block_hash: BlockHash,
    pub timestamp: u64,
    pub sequencer_address: Option<ContractAddress>,
    #[serde(default)]
    pub state_root: Option<Felt>,
    #[serde(default)]
    pub transaction_commitment: Option<Felt>,
    #[serde(default)]
    pub event_commitment: Option<Felt>,
    pub status: BlockStatus,
    pub l1_da_mode: L1DataAvailabilityMode,
    #[serde(default = "default_l2_gas_price")]
    pub l2_gas_price: ResourcePrice,
    pub l1_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    pub transactions: Vec<ConfirmedTransaction>,
    pub transaction_receipts: Vec<ConfirmedReceipt>,
    #[serde(default)]
    pub starknet_version: Option<StarknetVersion>,
}

// -- Conversion to Katana primitive types.

impl From<StateDiff> for katana_primitives::state::StateUpdates {
    fn from(value: StateDiff) -> Self {
        let storage_updates = value
            .storage_diffs
            .into_iter()
            .map(|(addr, diffs)| {
                let storage_map = diffs.into_iter().map(|diff| (diff.key, diff.value)).collect();
                (addr, storage_map)
            })
            .collect();

        let deployed_contracts = value
            .deployed_contracts
            .into_iter()
            .map(|contract| (contract.address, contract.class_hash))
            .collect();

        let declared_classes = value
            .declared_classes
            .into_iter()
            .map(|contract| (contract.class_hash, contract.compiled_class_hash))
            .collect();

        let replaced_classes = value
            .replaced_classes
            .into_iter()
            .map(|contract| (contract.address, contract.class_hash))
            .collect();

        Self {
            storage_updates,
            declared_classes,
            replaced_classes,
            deployed_contracts,
            nonce_updates: value.nonces,
            deprecated_declared_classes: BTreeSet::from_iter(value.old_declared_contracts),
        }
    }
}

fn default_l2_gas_price() -> ResourcePrice {
    ResourcePrice { price_in_fri: Felt::from(1), price_in_wei: Felt::from(1) }
}

#[cfg(test)]
mod tests {
    use katana_primitives::{address, felt, ContractAddress};

    use super::*;

    #[test]
    fn state_diff_to_state_updates_conversion() {
        let mut storage_diffs = BTreeMap::new();
        let contract_addr = address!("0x64");
        storage_diffs.insert(
            contract_addr,
            vec![
                StorageDiff { key: felt!("0x1"), value: felt!("0x2a") },
                StorageDiff { key: felt!("0x2"), value: felt!("0x2b") },
            ],
        );

        let deployed_contracts = vec![
            DeployedContract { address: address!("0xc8"), class_hash: felt!("0x12c") },
            DeployedContract { address: address!("0xc9"), class_hash: felt!("0x12d") },
        ];

        let declared_classes = vec![
            DeclaredContract { class_hash: felt!("0x190"), compiled_class_hash: felt!("0x1f4") },
            DeclaredContract { class_hash: felt!("0x191"), compiled_class_hash: felt!("0x1f5") },
        ];

        let old_declared_contracts = vec![felt!("0x258"), felt!("0x259")];

        let mut nonces = BTreeMap::new();
        nonces.insert(address!("0x2bc"), felt!("0x1"));
        nonces.insert(address!("0x2bd"), felt!("0x2"));

        let replaced_classes =
            vec![DeployedContract { address: address!("0x320"), class_hash: felt!("0x384") }];

        let state_diff = StateDiff {
            storage_diffs,
            deployed_contracts,
            old_declared_contracts,
            declared_classes,
            nonces,
            replaced_classes,
        };

        let state_updates: katana_primitives::state::StateUpdates = state_diff.into();

        // storage updates
        assert_eq!(state_updates.storage_updates.len(), 1);
        let storage_map = state_updates.storage_updates.get(&contract_addr).unwrap();
        assert_eq!(storage_map.len(), 2);
        assert_eq!(storage_map.get(&felt!("0x1")).unwrap(), &felt!("0x2a"));
        assert_eq!(storage_map.get(&felt!("0x2")).unwrap(), &felt!("0x2b"));

        // deployed contracts
        assert_eq!(state_updates.deployed_contracts.len(), 2);
        assert_eq!(
            state_updates.deployed_contracts.get(&address!("0xc8")).unwrap(),
            &felt!("0x12c")
        );
        assert_eq!(
            state_updates.deployed_contracts.get(&address!("0xc9")).unwrap(),
            &felt!("0x12d")
        );

        // declared classes
        assert_eq!(state_updates.declared_classes.len(), 2);
        assert_eq!(state_updates.declared_classes.get(&felt!("0x190")).unwrap(), &felt!("0x1f4"));
        assert_eq!(state_updates.declared_classes.get(&felt!("0x191")).unwrap(), &felt!("0x1f5"));

        // deprecated declared classes
        assert_eq!(state_updates.deprecated_declared_classes.len(), 2);
        assert!(state_updates.deprecated_declared_classes.contains(&felt!("0x258")));
        assert!(state_updates.deprecated_declared_classes.contains(&felt!("0x259")));

        // nonces
        assert_eq!(state_updates.nonce_updates.len(), 2);
        assert_eq!(state_updates.nonce_updates.get(&address!("0x2bc")).unwrap(), &felt!("0x1"));
        assert_eq!(state_updates.nonce_updates.get(&address!("0x2bd")).unwrap(), &felt!("0x2"));

        // replaced classes
        assert_eq!(state_updates.replaced_classes.len(), 1);
        assert_eq!(
            state_updates.replaced_classes.get(&address!("0x320")).unwrap(),
            &felt!("0x384")
        );
    }

    #[test]
    fn state_diff_empty_conversion() {
        let state_diff = StateDiff {
            storage_diffs: BTreeMap::new(),
            deployed_contracts: vec![],
            old_declared_contracts: vec![],
            declared_classes: vec![],
            nonces: BTreeMap::new(),
            replaced_classes: vec![],
        };

        let state_updates: katana_primitives::state::StateUpdates = state_diff.into();

        assert!(state_updates.storage_updates.is_empty());
        assert!(state_updates.deployed_contracts.is_empty());
        assert!(state_updates.declared_classes.is_empty());
        assert!(state_updates.deprecated_declared_classes.is_empty());
        assert!(state_updates.nonce_updates.is_empty());
        assert!(state_updates.replaced_classes.is_empty());
    }
}
