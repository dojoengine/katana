use std::collections::{BTreeMap, BTreeSet};
use std::iter;

use starknet_types_core::hash::{self, StarkHash};

use crate::cairo::ShortString;
use crate::class::{ClassHash, CompiledClassHash, ContractClass};
use crate::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use crate::Felt;

/// State updates.
///
/// Represents all the state updates after performing some executions on a state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StateUpdates {
    /// A mapping of contract addresses to their updated nonces.
    pub nonce_updates: BTreeMap<ContractAddress, Nonce>,
    /// A mapping of contract addresses to their updated storage entries.
    pub storage_updates: BTreeMap<ContractAddress, BTreeMap<StorageKey, StorageValue>>,
    /// A mapping of contract addresses to their updated class hashes.
    pub deployed_contracts: BTreeMap<ContractAddress, ClassHash>,
    /// A mapping of newly declared class hashes to their compiled class hashes.
    pub declared_classes: BTreeMap<ClassHash, CompiledClassHash>,
    /// A mapping of newly declared legacy class hashes.
    pub deprecated_declared_classes: BTreeSet<ClassHash>,
    /// A mapping of replaced contract addresses to their new class hashes ie using `replace_class`
    /// syscall.
    pub replaced_classes: BTreeMap<ContractAddress, ClassHash>,
    /// A list of classes whose compiled class hashes have been migrated.
    pub migrated_compiled_classes: BTreeMap<ClassHash, CompiledClassHash>,
}

impl StateUpdates {
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        let mut len: usize = 0;

        len += self.deployed_contracts.len();
        len += self.replaced_classes.len();
        len += self.declared_classes.len();
        len += self.deprecated_declared_classes.len();
        len += self.nonce_updates.len();
        len += self.migrated_compiled_classes.len();

        for updates in self.storage_updates.values() {
            len += updates.len();
        }

        len
    }
}

/// State update with declared classes artifacts.
#[derive(Debug, Default, Clone)]
pub struct StateUpdatesWithClasses {
    /// State updates.
    pub state_updates: StateUpdates,
    /// A mapping of class hashes to their sierra classes definition.
    pub classes: BTreeMap<ClassHash, ContractClass>,
}

impl StateUpdatesWithClasses {
    /// Validates that all declared classes have their corresponding class artifacts.
    ///
    /// This method checks that:
    /// - All class hashes in `state_updates.declared_classes` have entries in `classes`
    /// - All class hashes in `state_updates.deprecated_declared_classes` have entries in `classes`
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if all declared classes have their artifacts, otherwise returns an `Error`
    /// with the list of class hashes whose artifacts are missing.
    pub fn validate_classes(&self) -> Result<(), Vec<ClassHash>> {
        let mut missing = Vec::new();

        // Check declared classes
        for class_hash in self.state_updates.declared_classes.keys() {
            if !self.classes.contains_key(class_hash) {
                missing.push(*class_hash);
            }
        }

        // Check deprecated declared classes
        for class_hash in &self.state_updates.deprecated_declared_classes {
            if !self.classes.contains_key(class_hash) {
                missing.push(*class_hash);
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

pub fn compute_state_diff_hash(states: StateUpdates) -> Felt {
    let replaced_classes_len = states.replaced_classes.len();
    let deployed_contracts_len = states.deployed_contracts.len();
    let updated_contracts_len = Felt::from(deployed_contracts_len + replaced_classes_len);
    // flatten the updated contracts into a single list of Felt values
    let updated_contracts = states.deployed_contracts.into_iter().chain(states.replaced_classes);
    let updated_contracts = updated_contracts.flat_map(|(addr, hash)| vec![addr.into(), hash]);

    let declared_classes = states.declared_classes;
    let declared_classes_len = Felt::from(declared_classes.len());
    let declared_classes = declared_classes.into_iter().flat_map(|e| vec![e.0, e.1]);

    let deprecated_declared_classes = states.deprecated_declared_classes;
    let deprecated_declared_classes_len = Felt::from(deprecated_declared_classes.len());

    let storage_updates = states.storage_updates;
    let storage_updates_len = Felt::from(storage_updates.len());
    let storage_updates = storage_updates.into_iter().flat_map(|update| {
        let address = Felt::from(update.0);
        let storage_entries_len = Felt::from(update.1.len());
        let storage_entries = update.1.into_iter().flat_map(|entries| vec![entries.0, entries.1]);
        iter::once(address).chain(iter::once(storage_entries_len)).chain(storage_entries)
    });

    let nonce_updates = states.nonce_updates;
    let nonces_len = Felt::from(nonce_updates.len());
    let nonce_updates = nonce_updates.into_iter().flat_map(|nonce| vec![nonce.0.into(), nonce.1]);

    let magic = ShortString::from_ascii("STARKNET_STATE_DIFF0");
    let elements: Vec<Felt> = iter::once(Felt::from(magic))
        .chain(iter::once(updated_contracts_len))
        .chain(updated_contracts)
        .chain(iter::once(declared_classes_len))
        .chain(declared_classes)
        .chain(iter::once(deprecated_declared_classes_len))
        .chain(deprecated_declared_classes)
        .chain(iter::once(Felt::ONE))
        .chain(iter::once(Felt::ZERO))
        .chain(iter::once(storage_updates_len))
        .chain(storage_updates)
        .chain(iter::once(nonces_len))
        .chain(nonce_updates)
        .collect();

    hash::Poseidon::hash_array(&elements)
}
