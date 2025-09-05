use std::collections::{BTreeMap, BTreeSet};

use katana_primitives::block::BlockHash;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{Nonce, StorageKey, StorageValue};
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

/// Response object for the `starknet_getStateUpdate` RPC method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetStateUpdateResponse {
    pub state_update: MaybePreConfirmedStateUpdate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybePreConfirmedStateUpdate {
    Update(StateUpdate),
    PreConfirmed(PreConfirmedStateUpdate),
}

/// State update of a pre-confirmed block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreConfirmedStateUpdate {
    /// The previous global state root
    pub old_root: Felt,
    /// State diff
    pub state_diff: StateDiff,
}

/// State update of a confirmed block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateUpdate {
    /// Block hash
    pub block_hash: BlockHash,
    /// The previous global state root
    pub old_root: Felt,
    /// The new global state root
    pub new_root: Felt,
    /// State diff
    pub state_diff: StateDiff,
}

/// The change in state applied in this block, given as a mapping of addresses to the new values
/// and/or new contracts.
///
/// The side effect of using a [`BTreeMap`](std::collections::BTreeMap) is the entries will be
/// sorted by it's key in the resultant serialized JSON object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDiff {
    pub nonces: BTreeMap<ContractAddress, Nonce>,
    pub storage_diffs: BTreeMap<ContractAddress, BTreeMap<StorageKey, StorageValue>>,
    pub deployed_contracts: BTreeMap<ContractAddress, ClassHash>,
    pub declared_classes: BTreeMap<ClassHash, CompiledClassHash>,
    pub deprecated_declared_classes: BTreeSet<ClassHash>,
    pub replaced_classes: BTreeMap<ContractAddress, ClassHash>,
}

impl Serialize for StateDiff {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::{SerializeMap, SerializeSeq};

        /// Serializes nonces as an array of objects with the following structure:
        ///
        /// ```json
        /// [
        ///   {
        ///     "contract_address": "0x123",
        ///     "nonce": "0x123"
        ///   },
        ///   ...
        /// ]
        /// ```
        struct NoncesSer<'a>(&'a BTreeMap<ContractAddress, Nonce>);

        impl<'a> Serialize for NoncesSer<'a> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                #[derive(Debug, Serialize)]
                struct NonceUpdate {
                    contract_address: ContractAddress,
                    nonce: Nonce,
                }

                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for (contract_address, nonce) in self.0 {
                    seq.serialize_element(&NonceUpdate {
                        contract_address: *contract_address,
                        nonce: *nonce,
                    })?;
                }
                seq.end()
            }
        }

        /// Serializes storage diffs as an array of objects with the following structure:
        ///
        /// ```json
        /// [
        ///   {
        ///     "contract_address": "0x123",
        ///     "storage_entries": [
        ///       {
        ///         "key": "0x123",
        ///         "value": "0x456"
        ///       },
        ///       ...
        ///     ]
        ///   },
        ///   ...
        /// ]
        /// ```
        struct StorageDiffsSer<'a>(
            &'a BTreeMap<ContractAddress, BTreeMap<StorageKey, StorageValue>>,
        );

        impl<'a> Serialize for StorageDiffsSer<'a> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                #[derive(Debug, Serialize)]
                struct StorageEntry {
                    key: StorageKey,
                    value: StorageValue,
                }

                #[derive(Debug, Serialize)]
                struct ContractStorageDiff {
                    address: ContractAddress,
                    storage_entries: Vec<StorageEntry>,
                }

                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for (contract_address, entries) in self.0 {
                    let storage_entries: Vec<StorageEntry> = entries
                        .iter()
                        .map(|(key, value)| StorageEntry { key: *key, value: *value })
                        .collect();

                    seq.serialize_element(&ContractStorageDiff {
                        address: *contract_address,
                        storage_entries,
                    })?;
                }

                seq.end()
            }
        }

        /// Serializes deployed contracts as an array of objects with the following structure:
        ///
        /// ```json
        /// [
        ///   {
        ///     "address": "0x123",
        ///     "class_hash": "0x456"
        ///   },
        ///   ...
        /// ]
        /// ```
        struct DeployedContractsSer<'a>(&'a BTreeMap<ContractAddress, ClassHash>);

        impl<'a> Serialize for DeployedContractsSer<'a> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                #[derive(Debug, Serialize)]
                struct DeployedContract {
                    address: ContractAddress,
                    class_hash: ClassHash,
                }

                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for (contract_address, class_hash) in self.0 {
                    seq.serialize_element(&DeployedContract {
                        address: *contract_address,
                        class_hash: *class_hash,
                    })?;
                }
                seq.end()
            }
        }

        /// Serializes declared classes as an array of objects with the following structure:
        ///
        /// ```json
        /// [
        ///   {
        ///     "class_hash": "0x123",
        ///     "compiled_class_hash": "0x456"
        ///   },
        ///   ...
        /// ]
        /// ```
        struct DeclaredClassesSer<'a>(&'a BTreeMap<ClassHash, CompiledClassHash>);

        impl<'a> Serialize for DeclaredClassesSer<'a> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                #[derive(Debug, Serialize)]
                struct DeclaredClass {
                    class_hash: ClassHash,
                    compiled_class_hash: CompiledClassHash,
                }

                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for (class_hash, compiled_class_hash) in self.0 {
                    seq.serialize_element(&DeclaredClass {
                        class_hash: *class_hash,
                        compiled_class_hash: *compiled_class_hash,
                    })?;
                }
                seq.end()
            }
        }

        /// Serializes deprecated declared classes as an array of class hashes:
        ///
        /// ```json
        /// ["0x123", "0x456", ...]
        /// ```
        struct DepDeclaredClassesSer<'a>(&'a BTreeSet<ClassHash>);

        impl<'a> Serialize for DepDeclaredClassesSer<'a> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for class_hash in self.0 {
                    seq.serialize_element(class_hash)?;
                }
                seq.end()
            }
        }

        /// Serializes `replaced_classes` as an array of objects with the following structure:
        ///
        /// ```json
        /// [
        ///   {
        ///     "contract_address": "0x123",
        ///     "class_hash": "0x123"
        ///   },
        ///   ...
        /// ]
        /// ```
        struct ReplacedClassesSer<'a>(&'a BTreeMap<ContractAddress, ClassHash>);

        impl<'a> Serialize for ReplacedClassesSer<'a> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                #[derive(Debug, Serialize)]
                struct ReplacedClass {
                    contract_address: ContractAddress,
                    class_hash: ClassHash,
                }

                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for (contract_address, class_hash) in self.0 {
                    seq.serialize_element(&ReplacedClass {
                        contract_address: *contract_address,
                        class_hash: *class_hash,
                    })?;
                }
                seq.end()
            }
        }

        let nonces = NoncesSer(&self.nonces);
        let storage_diffs = StorageDiffsSer(&self.storage_diffs);
        let replaced_classes = ReplacedClassesSer(&self.replaced_classes);
        let declared_classes = DeclaredClassesSer(&self.declared_classes);
        let deployed_contracts = DeployedContractsSer(&self.deployed_contracts);
        let deprecated_declared_classes = DepDeclaredClassesSer(&self.deprecated_declared_classes);

        let mut map = serializer.serialize_map(Some(6))?;
        map.serialize_entry("nonces", &nonces)?;
        map.serialize_entry("storage_diffs", &storage_diffs)?;
        map.serialize_entry("declared_classes", &declared_classes)?;
        map.serialize_entry("replaced_classes", &replaced_classes)?;
        map.serialize_entry("deployed_contracts", &deployed_contracts)?;
        map.serialize_entry("deprecated_declared_classes", &deprecated_declared_classes)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for StateDiff {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{MapAccess, SeqAccess, Visitor};

        struct StateDiffVisitor;

        impl<'de> Visitor<'de> for StateDiffVisitor {
            type Value = StateDiff;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a valid StateDiff")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut nonces = None;
                let mut storage_diffs = None;
                let mut deployed_contracts = None;
                let mut declared_classes = None;
                let mut deprecated_declared_classes = None;
                let mut replaced_classes = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "nonces" => {
                            nonces = Some(map.next_value_seed(NoncesDe)?);
                        }
                        "storage_diffs" => {
                            storage_diffs = Some(map.next_value_seed(StorageDiffsDe)?);
                        }
                        "deployed_contracts" => {
                            deployed_contracts = Some(map.next_value_seed(DeployedContractsDe)?);
                        }
                        "declared_classes" => {
                            declared_classes = Some(map.next_value_seed(DeclaredClassesDe)?);
                        }
                        "deprecated_declared_classes" => {
                            deprecated_declared_classes =
                                Some(map.next_value_seed(DepDeclaredClassesDe)?);
                        }
                        "replaced_classes" => {
                            replaced_classes = Some(map.next_value_seed(ReplacedClassesDe)?);
                        }
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                Ok(StateDiff {
                    nonces: nonces.ok_or_else(|| serde::de::Error::missing_field("nonces"))?,
                    storage_diffs: storage_diffs
                        .ok_or_else(|| serde::de::Error::missing_field("storage_diffs"))?,
                    deployed_contracts: deployed_contracts
                        .ok_or_else(|| serde::de::Error::missing_field("deployed_contracts"))?,
                    declared_classes: declared_classes
                        .ok_or_else(|| serde::de::Error::missing_field("declared_classes"))?,
                    deprecated_declared_classes: deprecated_declared_classes.ok_or_else(|| {
                        serde::de::Error::missing_field("deprecated_declared_classes")
                    })?,
                    replaced_classes: replaced_classes
                        .ok_or_else(|| serde::de::Error::missing_field("replaced_classes"))?,
                })
            }
        }

        struct NoncesDe;

        impl<'de> serde::de::DeserializeSeed<'de> for NoncesDe {
            type Value = BTreeMap<ContractAddress, Nonce>;

            fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct NoncesVisitor;

                impl<'de> Visitor<'de> for NoncesVisitor {
                    type Value = BTreeMap<ContractAddress, Nonce>;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str("an array of nonce updates")
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                    where
                        A: SeqAccess<'de>,
                    {
                        #[derive(Debug, Deserialize)]
                        struct NonceUpdate {
                            contract_address: ContractAddress,
                            nonce: Nonce,
                        }

                        let mut nonces = BTreeMap::new();
                        while let Some(update) = seq.next_element::<NonceUpdate>()? {
                            nonces.insert(update.contract_address, update.nonce);
                        }
                        Ok(nonces)
                    }
                }

                deserializer.deserialize_seq(NoncesVisitor)
            }
        }

        struct StorageDiffsDe;

        impl<'de> serde::de::DeserializeSeed<'de> for StorageDiffsDe {
            type Value = BTreeMap<ContractAddress, BTreeMap<StorageKey, StorageValue>>;

            fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct StorageDiffsVisitor;

                impl<'de> Visitor<'de> for StorageDiffsVisitor {
                    type Value = BTreeMap<ContractAddress, BTreeMap<StorageKey, StorageValue>>;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str("an array of storage diffs")
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                    where
                        A: SeqAccess<'de>,
                    {
                        #[derive(Debug, Deserialize)]
                        struct StorageEntry {
                            key: StorageKey,
                            value: StorageValue,
                        }

                        #[derive(Debug, Deserialize)]
                        struct ContractStorageDiff {
                            address: ContractAddress,
                            storage_entries: Vec<StorageEntry>,
                        }

                        let mut storage_diffs = BTreeMap::new();
                        while let Some(diff) = seq.next_element::<ContractStorageDiff>()? {
                            let mut entries = BTreeMap::new();
                            for entry in diff.storage_entries {
                                entries.insert(entry.key, entry.value);
                            }
                            storage_diffs.insert(diff.address, entries);
                        }
                        Ok(storage_diffs)
                    }
                }

                deserializer.deserialize_seq(StorageDiffsVisitor)
            }
        }

        struct DeployedContractsDe;

        impl<'de> serde::de::DeserializeSeed<'de> for DeployedContractsDe {
            type Value = BTreeMap<ContractAddress, ClassHash>;

            fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct DeployedContractsVisitor;

                impl<'de> Visitor<'de> for DeployedContractsVisitor {
                    type Value = BTreeMap<ContractAddress, ClassHash>;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str("an array of deployed contracts")
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                    where
                        A: SeqAccess<'de>,
                    {
                        #[derive(Debug, Deserialize)]
                        struct DeployedContract {
                            address: ContractAddress,
                            class_hash: ClassHash,
                        }

                        let mut deployed_contracts = BTreeMap::new();
                        while let Some(contract) = seq.next_element::<DeployedContract>()? {
                            deployed_contracts.insert(contract.address, contract.class_hash);
                        }
                        Ok(deployed_contracts)
                    }
                }

                deserializer.deserialize_seq(DeployedContractsVisitor)
            }
        }

        struct DeclaredClassesDe;

        impl<'de> serde::de::DeserializeSeed<'de> for DeclaredClassesDe {
            type Value = BTreeMap<ClassHash, CompiledClassHash>;

            fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct DeclaredClassesVisitor;

                impl<'de> Visitor<'de> for DeclaredClassesVisitor {
                    type Value = BTreeMap<ClassHash, CompiledClassHash>;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str("an array of declared classes")
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                    where
                        A: SeqAccess<'de>,
                    {
                        #[derive(Debug, Deserialize)]
                        struct DeclaredClass {
                            class_hash: ClassHash,
                            compiled_class_hash: CompiledClassHash,
                        }

                        let mut declared_classes = BTreeMap::new();
                        while let Some(class) = seq.next_element::<DeclaredClass>()? {
                            declared_classes.insert(class.class_hash, class.compiled_class_hash);
                        }
                        Ok(declared_classes)
                    }
                }

                deserializer.deserialize_seq(DeclaredClassesVisitor)
            }
        }

        struct DepDeclaredClassesDe;

        impl<'de> serde::de::DeserializeSeed<'de> for DepDeclaredClassesDe {
            type Value = BTreeSet<ClassHash>;

            fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct DepDeclaredClassesVisitor;

                impl<'de> Visitor<'de> for DepDeclaredClassesVisitor {
                    type Value = BTreeSet<ClassHash>;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str("an array of class hashes")
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                    where
                        A: SeqAccess<'de>,
                    {
                        let mut deprecated_declared_classes = BTreeSet::new();
                        while let Some(class_hash) = seq.next_element::<ClassHash>()? {
                            deprecated_declared_classes.insert(class_hash);
                        }
                        Ok(deprecated_declared_classes)
                    }
                }

                deserializer.deserialize_seq(DepDeclaredClassesVisitor)
            }
        }

        struct ReplacedClassesDe;

        impl<'de> serde::de::DeserializeSeed<'de> for ReplacedClassesDe {
            type Value = BTreeMap<ContractAddress, ClassHash>;

            fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct ReplacedClassesVisitor;

                impl<'de> Visitor<'de> for ReplacedClassesVisitor {
                    type Value = BTreeMap<ContractAddress, ClassHash>;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str("an array of replaced classes")
                    }

                    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                    where
                        A: SeqAccess<'de>,
                    {
                        #[derive(Debug, Deserialize)]
                        struct ReplacedClass {
                            contract_address: ContractAddress,
                            class_hash: ClassHash,
                        }

                        let mut replaced_classes = BTreeMap::new();
                        while let Some(replaced) = seq.next_element::<ReplacedClass>()? {
                            replaced_classes.insert(replaced.contract_address, replaced.class_hash);
                        }
                        Ok(replaced_classes)
                    }
                }

                deserializer.deserialize_seq(ReplacedClassesVisitor)
            }
        }

        deserializer.deserialize_map(StateDiffVisitor)
    }
}

impl From<katana_primitives::state::StateUpdates> for StateDiff {
    fn from(value: katana_primitives::state::StateUpdates) -> Self {
        Self {
            nonces: value.nonce_updates,
            replaced_classes: Default::default(),
            storage_diffs: value.storage_updates,
            declared_classes: value.declared_classes,
            deployed_contracts: value.deployed_contracts,
            deprecated_declared_classes: value.deprecated_declared_classes,
        }
    }
}
