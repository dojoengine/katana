//! JSON representation of the genesis configuration. Used to deserialize the genesis configuration
//! from a JSON file.

use std::collections::{btree_map, hash_map, BTreeMap, HashMap};
use std::fs::File;
use std::io::{
    BufReader, {self},
};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use alloy_primitives::U256;
use base64::prelude::*;
use cairo_vm::types::errors::program_errors::ProgramError;
use serde::de::value::MapAccessDeserializer;
use serde::de::Visitor;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use starknet::core::types::contract::JsonError;

use super::allocation::{
    DevGenesisAccount, GenesisAccount, GenesisAccountAlloc, GenesisContractAlloc,
};
use super::constant::{DEFAULT_ACCOUNT_CLASS, DEFAULT_ACCOUNT_CLASS_HASH};
use super::{Genesis, GenesisAllocation};
use crate::block::{BlockHash, BlockNumber, GasPrices};
use crate::class::{
    ClassHash, ComputeClassHashError, ContractClass, ContractClassCompilationError,
    LegacyContractClass, SierraContractClass,
};
use crate::contract::{ContractAddress, StorageKey, StorageValue};
use crate::Felt;

type Object = Map<String, Value>;

/// Represents the path to the class artifact or the full JSON artifact itself.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, derive_more::From)]
#[serde(untagged)]
pub enum PathOrFullArtifact {
    /// A path to the file.
    Path(PathBuf),
    /// The full JSON artifact.
    Artifact(Value),
}

impl<'de> Deserialize<'de> for PathOrFullArtifact {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct _Visitor;

        impl<'de> Visitor<'de> for _Visitor {
            type Value = PathOrFullArtifact;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a path to a file or the full json artifact")
            }

            fn visit_str<E>(self, v: &str) -> Result<PathOrFullArtifact, E>
            where
                E: serde::de::Error,
            {
                Ok(PathOrFullArtifact::Path(PathBuf::from(v)))
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                Ok(PathOrFullArtifact::Artifact(Value::Object(Object::deserialize(
                    MapAccessDeserializer::new(map),
                )?)))
            }
        }

        deserializer.deserialize_any(_Visitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenesisClassJson {
    // pub class: PathBuf,
    pub class: PathOrFullArtifact,
    // Allows class identification by a unique name rather than by hash when specifying the class.
    pub name: Option<String>,
}

/// Class identifier.
///
/// When deploying a contract through the genesis file, the class implementation of the contract
/// can be specified either by the class hash or by the class name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassNameOrHash {
    Name(String),
    Hash(ClassHash),
}

// We implement the `Serialize` manually because we need to serialize the class hash as hex string
// with the `0x` prefix (if it's the hash variant). Otherwise, it'd be a decimal string, and
// deserializing it back would fail as it'd be interpreted as a name instead.
impl Serialize for ClassNameOrHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ClassNameOrHash::Name(name) => serializer.serialize_str(name),
            ClassNameOrHash::Hash(hash) => serializer.serialize_str(&format!("{hash:#x}")),
        }
    }
}

impl<'de> Deserialize<'de> for ClassNameOrHash {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct _Visitor;

        impl Visitor<'_> for _Visitor {
            type Value = ClassNameOrHash;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a name or a class hash prefixed with 0x")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v.starts_with("0x") {
                    Ok(ClassNameOrHash::Hash(ClassHash::from_str(v).map_err(E::custom)?))
                } else {
                    Ok(ClassNameOrHash::Name(v.to_string()))
                }
            }
        }

        deserializer.deserialize_any(_Visitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GenesisContractJson {
    pub class: Option<ClassNameOrHash>,
    pub balance: Option<U256>,
    pub nonce: Option<Felt>,
    pub storage: Option<BTreeMap<StorageKey, StorageValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GenesisAccountJson {
    /// The public key of the account.
    pub public_key: Felt,
    pub balance: Option<U256>,
    pub nonce: Option<Felt>,
    /// The class hash of the account contract. If not provided, the default account class is used.
    pub class: Option<ClassNameOrHash>,
    pub storage: Option<BTreeMap<StorageKey, StorageValue>>,
    pub private_key: Option<Felt>,
    pub salt: Option<Felt>,
}

#[derive(Debug, thiserror::Error)]
pub enum GenesisJsonError {
    #[error("Failed to read class file at path {path}: {source}")]
    FileNotFound { source: io::Error, path: PathBuf },

    #[error(transparent)]
    ParsingError(#[from] serde_json::Error),

    #[error(transparent)]
    ComputeClassHash(#[from] ComputeClassHashError),

    #[error(transparent)]
    ProgramError(#[from] ProgramError),

    #[error("Missing class entry for class hash {0}")]
    MissingClass(ClassHash),

    #[error("Failed to flatten Sierra contract: {0}")]
    FlattenSierraClass(#[from] JsonError),

    #[error("Unresolved class artifact path {0}")]
    UnresolvedClassPath(PathBuf),

    #[error(transparent)]
    Encode(#[from] base64::EncodeSliceError),

    #[error(transparent)]
    Decode(#[from] base64::DecodeError),

    #[error("Class name '{0}' already exists in the genesis classes")]
    DuplicateClassName(String),

    #[error("Class name '{0}' not found in the genesis classes")]
    UnknownClassName(String),

    #[error(transparent)]
    ContractClassCompilation(#[from] ContractClassCompilationError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// The JSON representation of the [Genesis] configuration. This `struct` is used to deserialize
/// the genesis configuration from a JSON file before being converted to a [Genesis] instance.
///
/// The JSON format allows specifying either the path to the class artifact or the full artifact
/// embedded directly inside the JSON file. As such, it is required that all paths must be resolved
/// first before converting to [Genesis] using [`Genesis::try_from<GenesisJson>`], otherwise the
/// conversion will fail.
///
/// It is recommended to use [GenesisJson::load] for loading the JSON file as it will resolve
/// the class paths into their actual class artifacts, instead of deserializing it manually
/// (eg, using `serde_json`).
///
/// The path of the class artifact are computed **relative** to the JSON file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenesisJson {
    pub parent_hash: BlockHash,
    pub state_root: Felt,
    pub number: BlockNumber,
    pub timestamp: u64,
    pub sequencer_address: ContractAddress,
    pub gas_prices: GasPrices,
    #[serde(default)]
    pub classes: Vec<GenesisClassJson>,
    #[serde(default)]
    pub accounts: BTreeMap<ContractAddress, GenesisAccountJson>,
    #[serde(default)]
    pub contracts: BTreeMap<ContractAddress, GenesisContractJson>,
}

impl GenesisJson {
    /// Load the genesis configuration from a JSON file at the given `path` and resolve all the
    /// class paths to their corresponding class definitions. The paths will be resolved relative
    /// to the JSON file itself.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, GenesisJsonError> {
        let mut path = path.as_ref().to_path_buf();

        let file = File::open(&path)
            .map_err(|source| GenesisJsonError::FileNotFound { path: path.clone(), source })?;

        // Remove the file name from the path to get the base path.
        path.pop();

        let mut genesis: Self = serde_json::from_reader(BufReader::new(file))?;
        // resolves the class paths, if any
        genesis.resolve_class_artifacts(path)?;

        Ok(genesis)
    }

    /// Resolves the paths of the class files to their corresponding class definitions. The
    /// `base_path` is used to calculate the paths of the class files, which are relative to the
    /// JSON file itself.
    ///
    /// This needs to be called if the [GenesisJson] is instantiated without using the
    /// [GenesisJson::load] before converting to [Genesis].
    pub fn resolve_class_artifacts(
        &mut self,
        base_path: impl AsRef<Path>,
    ) -> Result<(), GenesisJsonError> {
        for entry in &mut self.classes {
            if let PathOrFullArtifact::Path(rel_path) = &entry.class {
                let base_path = base_path.as_ref().to_path_buf();
                let artifact = class_artifact_at_path(base_path, rel_path)?;
                entry.class = PathOrFullArtifact::Artifact(artifact);
            }
        }
        Ok(())
    }
}

impl TryFrom<GenesisJson> for Genesis {
    type Error = GenesisJsonError;

    fn try_from(value: GenesisJson) -> Result<Self, Self::Error> {
        // a lookup table for classes that is assigned a name
        let mut class_names: HashMap<String, Felt> = HashMap::new();
        let mut classes: BTreeMap<ClassHash, Arc<ContractClass>> = BTreeMap::new();

        for entry in value.classes {
            let GenesisClassJson { class, name } = entry;

            // at this point, it is assumed that any class paths should have been resolved to an
            // artifact, otherwise it is an error
            let artifact = match class {
                PathOrFullArtifact::Artifact(artifact) => artifact,
                PathOrFullArtifact::Path(path) => {
                    return Err(GenesisJsonError::UnresolvedClassPath(path));
                }
            };

            let sierra = serde_json::from_value::<SierraContractClass>(artifact.clone());

            let (class_hash, class) = match sierra {
                Ok(sierra) => {
                    // check if the class hash is provided, otherwise compute it from the
                    // artifacts
                    let class = ContractClass::Class(sierra);
                    let class_hash = class.class_hash()?;

                    (class_hash, Arc::new(class))
                }

                // if the artifact is not a sierra contract, we check if it's a legacy contract
                Err(_) => {
                    let casm = serde_json::from_value::<LegacyContractClass>(artifact.clone())?;

                    let casm = ContractClass::Legacy(casm);
                    let class_hash = casm.class_hash()?;

                    (class_hash, Arc::new(casm))
                }
            };

            // if the class has a name, we add it to the lookup table to use later when we're
            // parsing the contracts
            if let Some(name) = name {
                // if there's a duplicate class name, we return an error
                match class_names.entry(name.clone()) {
                    hash_map::Entry::Occupied(_) => {
                        return Err(GenesisJsonError::DuplicateClassName(name));
                    }

                    hash_map::Entry::Vacant(e) => {
                        e.insert(class_hash);
                    }
                }
            }

            classes.insert(class_hash, class);
        }

        let mut allocations: BTreeMap<ContractAddress, GenesisAllocation> = BTreeMap::new();

        for (address, account) in value.accounts {
            // check that the class hash exists in the classes field
            let class_hash = match account.class {
                Some(class) => {
                    let hash = match class {
                        ClassNameOrHash::Hash(hash) => hash,
                        ClassNameOrHash::Name(name) => {
                            // Handle the case when the class is specified by name.
                            *class_names
                                .get(&name)
                                .ok_or_else(|| GenesisJsonError::UnknownClassName(name))?
                        }
                    };

                    if !classes.contains_key(&hash) {
                        return Err(GenesisJsonError::MissingClass(hash));
                    } else {
                        hash
                    }
                }

                None => {
                    // check that the default account class exists in the classes field before
                    // inserting it
                    if let btree_map::Entry::Vacant(e) = classes.entry(DEFAULT_ACCOUNT_CLASS_HASH) {
                        // insert default account class to the classes map
                        e.insert(DEFAULT_ACCOUNT_CLASS.clone().into());
                    }

                    DEFAULT_ACCOUNT_CLASS_HASH
                }
            };

            match account.private_key {
                Some(private_key) => {
                    let mut inner = if let Some(salt) = account.salt {
                        GenesisAccount::new_with_salt(account.public_key, class_hash, salt)
                    } else {
                        GenesisAccount::new(account.public_key, class_hash)
                    };

                    inner.nonce = account.nonce;
                    inner.storage = account.storage;
                    inner.balance = account.balance;

                    allocations.insert(
                        address,
                        GenesisAllocation::Account(GenesisAccountAlloc::DevAccount(
                            DevGenesisAccount { private_key, inner },
                        )),
                    )
                }

                None => {
                    let mut inner = if let Some(salt) = account.salt {
                        GenesisAccount::new_with_salt(account.public_key, class_hash, salt)
                    } else {
                        GenesisAccount::new(account.public_key, class_hash)
                    };

                    inner.nonce = account.nonce;
                    inner.storage = account.storage;
                    inner.balance = account.balance;

                    allocations.insert(
                        address,
                        GenesisAllocation::Account(GenesisAccountAlloc::Account(inner)),
                    )
                }
            };
        }

        for (address, contract) in value.contracts {
            // check that the class hash exists in the classes field
            let class_hash = if let Some(class) = contract.class {
                let hash = match class {
                    ClassNameOrHash::Hash(hash) => hash,
                    ClassNameOrHash::Name(name) => {
                        // Handle the case when the class is specified by name.
                        *class_names
                            .get(&name)
                            .ok_or_else(|| GenesisJsonError::UnknownClassName(name))?
                    }
                };

                Some(hash)
            } else {
                None
            };

            if let Some(hash) = class_hash {
                if !classes.contains_key(&hash) {
                    return Err(GenesisJsonError::MissingClass(hash));
                }
            }

            allocations.insert(
                address,
                GenesisAllocation::Contract(GenesisContractAlloc {
                    balance: contract.balance,
                    class_hash,
                    nonce: contract.nonce,
                    storage: contract.storage,
                }),
            );
        }

        Ok(Genesis {
            classes,
            allocations,
            number: value.number,
            sequencer_address: value.sequencer_address,
            timestamp: value.timestamp,
            gas_prices: value.gas_prices,
            state_root: value.state_root,
            parent_hash: value.parent_hash,
        })
    }
}

impl TryFrom<Genesis> for GenesisJson {
    type Error = GenesisJsonError;

    fn try_from(value: Genesis) -> Result<Self, Self::Error> {
        let mut contracts = BTreeMap::new();
        let mut accounts = BTreeMap::new();
        let mut classes = Vec::with_capacity(value.classes.len());

        for (.., class) in value.classes {
            // Convert the class to an artifact Value
            let artifact = match class.as_ref() {
                ContractClass::Legacy(casm) => serde_json::to_value(casm)?,
                ContractClass::Class(sierra) => serde_json::to_value(sierra)?,
            };

            classes.push(GenesisClassJson {
                class: PathOrFullArtifact::Artifact(artifact),
                name: None,
            });
        }

        for (address, allocation) in value.allocations {
            match allocation {
                GenesisAllocation::Account(account) => match account {
                    GenesisAccountAlloc::Account(acc) => {
                        accounts.insert(
                            address,
                            GenesisAccountJson {
                                nonce: acc.nonce,
                                private_key: None,
                                salt: Some(acc.salt),
                                storage: acc.storage,
                                balance: acc.balance,
                                public_key: acc.public_key,
                                class: Some(ClassNameOrHash::Hash(acc.class_hash)),
                            },
                        );
                    }
                    GenesisAccountAlloc::DevAccount(dev_acc) => {
                        accounts.insert(
                            address,
                            GenesisAccountJson {
                                salt: Some(dev_acc.salt),
                                nonce: dev_acc.inner.nonce,
                                balance: dev_acc.inner.balance,
                                storage: dev_acc.inner.storage,
                                public_key: dev_acc.inner.public_key,
                                private_key: Some(dev_acc.private_key),
                                class: Some(ClassNameOrHash::Hash(dev_acc.inner.class_hash)),
                            },
                        );
                    }
                },
                GenesisAllocation::Contract(contract) => {
                    contracts.insert(
                        address,
                        GenesisContractJson {
                            nonce: contract.nonce,
                            balance: contract.balance,
                            storage: contract.storage,
                            class: contract.class_hash.map(ClassNameOrHash::Hash),
                        },
                    );
                }
            }
        }

        Ok(GenesisJson {
            parent_hash: value.parent_hash,
            state_root: value.state_root,
            number: value.number,
            timestamp: value.timestamp,
            sequencer_address: value.sequencer_address,
            gas_prices: value.gas_prices,
            classes,
            accounts,
            contracts,
        })
    }
}

impl FromStr for GenesisJson {
    type Err = GenesisJsonError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s).map_err(GenesisJsonError::from)
    }
}

/// A helper function to conveniently resolve the artifacts in the genesis json if they
/// weren't already resolved and then serialize it to base64 encoding.
///
/// # Arguments
/// * `genesis` - The [GenesisJson] to resolve and serialize.
/// * `base_path` - The base path of the JSON file used to resolve the class artifacts
pub fn resolve_artifacts_and_to_base64<P: AsRef<Path>>(
    mut genesis: GenesisJson,
    base_path: P,
) -> Result<Vec<u8>, GenesisJsonError> {
    genesis.resolve_class_artifacts(base_path)?;
    to_base64(genesis)
}

/// Serialize the [GenesisJson] into base64 encoding.
pub fn to_base64(genesis: GenesisJson) -> Result<Vec<u8>, GenesisJsonError> {
    let data = serde_json::to_vec(&genesis)?;

    // make sure we'll have a slice big enough for base64 + padding
    let mut buf = vec![0; (4 * data.len() / 3) + 4];

    let bytes_written = BASE64_STANDARD.encode_slice(data, &mut buf)?;
    // shorten the buffer to the actual length written
    buf.truncate(bytes_written);

    Ok(buf)
}

/// Deserialize the [GenesisJson] from base64 encoded bytes.
pub fn from_base64(data: &[u8]) -> Result<GenesisJson, GenesisJsonError> {
    let decoded = BASE64_STANDARD.decode(data)?;
    Ok(serde_json::from_slice::<GenesisJson>(&decoded)?)
}

fn class_artifact_at_path(
    base_path: PathBuf,
    relative_path: &PathBuf,
) -> Result<serde_json::Value, GenesisJsonError> {
    let mut path = base_path;
    path.push(relative_path);

    let path =
        path.canonicalize().map_err(|e| GenesisJsonError::FileNotFound { source: e, path })?;

    let file = File::open(&path).map_err(|e| GenesisJsonError::FileNotFound { source: e, path })?;
    let content: Value = serde_json::from_reader(BufReader::new(file))?;

    Ok(content)
}

#[cfg(test)]
mod tests {
    use starknet::macros::felt;

    use super::*;
    use crate::address;
    use crate::genesis::constant::{
        DEFAULT_LEGACY_ERC20_CLASS, DEFAULT_LEGACY_ERC20_CLASS_HASH, DEFAULT_LEGACY_UDC_CLASS,
        DEFAULT_LEGACY_UDC_CLASS_HASH,
    };

    #[test]
    fn deserialize_from_json() {
        let file = File::open("./src/genesis/test-genesis.json").unwrap();
        let json: GenesisJson = serde_json::from_reader(file).unwrap();

        assert_eq!(json.number, 0);
        assert_eq!(json.parent_hash, felt!("0x999"));
        assert_eq!(json.timestamp, 5123512314u64);
        assert_eq!(json.state_root, felt!("0x99"));
        assert_eq!(json.gas_prices.eth.get(), 1111);
        assert_eq!(json.gas_prices.strk.get(), 2222);

        let acc_1 = address!("0x66efb28ac62686966ae85095ff3a772e014e7fbf56d4c5f6fac5606d4dde23a");
        let acc_2 = address!("0x6b86e40118f29ebe393a75469b4d926c7a44c2e2681b6d319520b7c1156d114");
        let acc_3 = address!("0x79156ecb3d8f084001bb498c95e37fa1c4b40dbb35a3ae47b77b1ad535edcb9");
        let acc_4 = address!("0x053a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf");

        assert_eq!(json.accounts.len(), 4);

        assert_eq!(json.accounts[&acc_1].public_key, felt!("0x1"));
        assert_eq!(
            json.accounts[&acc_1].balance,
            Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap())
        );
        assert_eq!(json.accounts[&acc_1].nonce, Some(felt!("0x1")));
        assert_eq!(json.accounts[&acc_1].class, Some(ClassNameOrHash::Name("Foo".to_string())));
        assert_eq!(
            json.accounts[&acc_1].storage,
            Some(BTreeMap::from([(felt!("0x1"), felt!("0x1")), (felt!("0x2"), felt!("0x2")),]))
        );

        assert_eq!(json.accounts[&acc_2].public_key, felt!("0x2"));
        assert_eq!(
            json.accounts[&acc_2].balance,
            Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap())
        );
        assert_eq!(json.accounts[&acc_2].nonce, None);
        assert_eq!(json.accounts[&acc_2].class, Some(ClassNameOrHash::Name("MyClass".to_string())));
        assert_eq!(json.accounts[&acc_2].storage, None);

        assert_eq!(json.accounts[&acc_3].public_key, felt!("0x3"));
        assert_eq!(json.accounts[&acc_3].balance, None);
        assert_eq!(json.accounts[&acc_3].nonce, None);
        assert_eq!(json.accounts[&acc_3].class, None);
        assert_eq!(json.accounts[&acc_3].storage, None);

        assert_eq!(json.accounts[&acc_4].public_key, felt!("0x4"));
        assert_eq!(json.accounts[&acc_4].private_key.unwrap(), felt!("0x115"));
        assert_eq!(
            json.accounts[&acc_4].balance,
            Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap())
        );
        assert_eq!(json.accounts[&acc_4].nonce, None);
        assert_eq!(json.accounts[&acc_4].class, None);
        assert_eq!(json.accounts[&acc_4].storage, None);

        assert_eq!(json.contracts.len(), 3);

        let contract_1 =
            address!("0x29873c310fbefde666dc32a1554fea6bb45eecc84f680f8a2b0a8fbb8cb89af");
        let contract_2 =
            address!("0xe29882a1fcba1e7e10cad46212257fea5c752a4f9b1b1ec683c503a2cf5c8a");
        let contract_3 =
            address!("0x05400e90f7e0ae78bd02c77cd75527280470e2fe19c54970dd79dc37a9d3645c");

        assert_eq!(
            json.contracts[&contract_1].balance,
            Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap())
        );
        assert_eq!(json.contracts[&contract_1].nonce, None);
        assert_eq!(
            json.contracts[&contract_1].class,
            Some(ClassNameOrHash::Name(String::from("MyErc20")))
        );
        assert_eq!(
            json.contracts[&contract_1].storage,
            Some(BTreeMap::from([(felt!("0x1"), felt!("0x1")), (felt!("0x2"), felt!("0x2"))]))
        );

        assert_eq!(
            json.contracts[&contract_2].balance,
            Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap())
        );
        assert_eq!(json.contracts[&contract_2].nonce, None);
        assert_eq!(json.contracts[&contract_2].class, None);
        assert_eq!(json.contracts[&contract_2].storage, None);

        assert_eq!(json.contracts[&contract_3].balance, None);
        assert_eq!(json.contracts[&contract_3].nonce, None);
        assert_eq!(
            json.contracts[&contract_3].class,
            Some(ClassNameOrHash::Name("Foo".to_string()))
        );
        assert_eq!(
            json.contracts[&contract_3].storage,
            Some(BTreeMap::from([(felt!("0x1"), felt!("0x1"))]))
        );

        similar_asserts::assert_eq!(
            json.classes,
            vec![
                GenesisClassJson {
                    class: PathBuf::from("../../../contracts/build/legacy/erc20.json").into(),
                    name: Some("MyErc20".to_string()),
                },
                GenesisClassJson {
                    class: PathBuf::from("../../../contracts/build/legacy/universal_deployer.json")
                        .into(),
                    name: Some("Foo".to_string()),
                },
                GenesisClassJson {
                    class: PathBuf::from(
                        "../../../contracts/build/katana_account_Account.contract_class.json"
                    )
                    .into(),
                    name: Some("MyClass".to_string()),
                },
            ]
        );
    }

    #[test]
    fn deserialize_from_json_with_class() {
        let file = File::open("./src/genesis/test-genesis-with-class.json").unwrap();
        let genesis: GenesisJson = serde_json::from_reader(BufReader::new(file)).unwrap();
        similar_asserts::assert_eq!(
            genesis.classes,
            vec![
                GenesisClassJson {
                    class: PathBuf::from("../../../contracts/build/legacy/erc20.json").into(),
                    name: Some("MyErc20".to_string()),
                },
                GenesisClassJson {
                    class: PathBuf::from("../../../contracts/build/legacy/universal_deployer.json")
                        .into(),
                    name: Some("Foo".to_string()),
                },
                GenesisClassJson {
                    class: serde_json::to_value(DEFAULT_ACCOUNT_CLASS.as_sierra().unwrap())
                        .unwrap()
                        .into(),
                    name: None,
                },
            ]
        );
    }

    #[test]
    fn genesis_load_from_json() {
        let path = PathBuf::from("./src/genesis/test-genesis.json");

        let json = GenesisJson::load(path).unwrap();
        let actual_genesis = Genesis::try_from(json).unwrap();

        let mut expected_classes = BTreeMap::new();

        expected_classes
            .insert(DEFAULT_LEGACY_ERC20_CLASS_HASH, DEFAULT_LEGACY_ERC20_CLASS.clone().into());
        expected_classes
            .insert(DEFAULT_LEGACY_UDC_CLASS_HASH, DEFAULT_LEGACY_UDC_CLASS.clone().into());
        expected_classes.insert(DEFAULT_ACCOUNT_CLASS_HASH, DEFAULT_ACCOUNT_CLASS.clone().into());

        let acc_1 = address!("0x66efb28ac62686966ae85095ff3a772e014e7fbf56d4c5f6fac5606d4dde23a");
        let acc_2 = address!("0x6b86e40118f29ebe393a75469b4d926c7a44c2e2681b6d319520b7c1156d114");
        let acc_3 = address!("0x79156ecb3d8f084001bb498c95e37fa1c4b40dbb35a3ae47b77b1ad535edcb9");
        let acc_4 = address!("0x053a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf");
        let contract_1 =
            address!("0x29873c310fbefde666dc32a1554fea6bb45eecc84f680f8a2b0a8fbb8cb89af");
        let contract_2 =
            address!("0xe29882a1fcba1e7e10cad46212257fea5c752a4f9b1b1ec683c503a2cf5c8a");
        let contract_3 =
            address!("0x05400e90f7e0ae78bd02c77cd75527280470e2fe19c54970dd79dc37a9d3645c");

        let expected_allocations = BTreeMap::from([
            (
                acc_1,
                GenesisAllocation::Account(GenesisAccountAlloc::Account(GenesisAccount {
                    public_key: felt!("0x1"),
                    balance: Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap()),
                    nonce: Some(felt!("0x1")),
                    class_hash: DEFAULT_LEGACY_UDC_CLASS_HASH,
                    storage: Some(BTreeMap::from([
                        (felt!("0x1"), felt!("0x1")),
                        (felt!("0x2"), felt!("0x2")),
                    ])),
                    salt: GenesisAccount::DEFAULT_SALT,
                })),
            ),
            (
                acc_2,
                GenesisAllocation::Account(GenesisAccountAlloc::Account(GenesisAccount {
                    public_key: felt!("0x2"),
                    balance: Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap()),
                    class_hash: DEFAULT_ACCOUNT_CLASS_HASH,
                    nonce: None,
                    storage: None,
                    salt: GenesisAccount::DEFAULT_SALT,
                })),
            ),
            (
                acc_3,
                GenesisAllocation::Account(GenesisAccountAlloc::Account(GenesisAccount {
                    public_key: felt!("0x3"),
                    balance: None,
                    class_hash: DEFAULT_ACCOUNT_CLASS_HASH,
                    nonce: None,
                    storage: None,
                    salt: GenesisAccount::DEFAULT_SALT,
                })),
            ),
            (
                acc_4,
                GenesisAllocation::Account(GenesisAccountAlloc::DevAccount(DevGenesisAccount {
                    private_key: felt!("0x115"),
                    inner: GenesisAccount {
                        public_key: felt!("0x4"),
                        balance: Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap()),
                        class_hash: DEFAULT_ACCOUNT_CLASS_HASH,
                        nonce: None,
                        storage: None,
                        salt: GenesisAccount::DEFAULT_SALT,
                    },
                })),
            ),
            (
                contract_1,
                GenesisAllocation::Contract(GenesisContractAlloc {
                    balance: Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap()),
                    nonce: None,
                    class_hash: Some(DEFAULT_LEGACY_ERC20_CLASS_HASH),
                    storage: Some(BTreeMap::from([
                        (felt!("0x1"), felt!("0x1")),
                        (felt!("0x2"), felt!("0x2")),
                    ])),
                }),
            ),
            (
                contract_2,
                GenesisAllocation::Contract(GenesisContractAlloc {
                    balance: Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap()),
                    nonce: None,
                    class_hash: None,
                    storage: None,
                }),
            ),
            (
                contract_3,
                GenesisAllocation::Contract(GenesisContractAlloc {
                    balance: None,
                    nonce: None,
                    class_hash: Some(DEFAULT_LEGACY_UDC_CLASS_HASH),
                    storage: Some(BTreeMap::from([(felt!("0x1"), felt!("0x1"))])),
                }),
            ),
        ]);

        let expected_genesis = Genesis {
            classes: expected_classes,
            number: 0,
            // fee_token: expected_fee_token,
            allocations: expected_allocations,
            timestamp: 5123512314u64,
            sequencer_address: address!("0x100"),
            state_root: felt!("0x99"),
            parent_hash: felt!("0x999"),
            gas_prices: unsafe { GasPrices::new_unchecked(1111, 2222) },
        };

        assert_eq!(actual_genesis.number, expected_genesis.number);
        assert_eq!(actual_genesis.parent_hash, expected_genesis.parent_hash);
        assert_eq!(actual_genesis.timestamp, expected_genesis.timestamp);
        assert_eq!(actual_genesis.state_root, expected_genesis.state_root);
        assert_eq!(actual_genesis.gas_prices, expected_genesis.gas_prices);
        assert_eq!(actual_genesis.allocations.len(), expected_genesis.allocations.len());

        for alloc in actual_genesis.allocations {
            let expected_alloc = expected_genesis.allocations.get(&alloc.0).unwrap();
            assert_eq!(alloc.1, *expected_alloc);
        }

        assert_eq!(actual_genesis.classes.len(), expected_genesis.classes.len());

        for class in actual_genesis.classes {
            let expected_class = expected_genesis.classes.get(&class.0).unwrap();
            assert_eq!(&class.1, expected_class);
        }
    }

    // We don't care what the intermediate JSON format looks like as long as the
    // conversion back and forth between GenesisJson and Genesis results in equivalent Genesis
    // structs
    #[test]
    fn genesis_conversion_rt() {
        let path = PathBuf::from("./src/genesis/test-genesis.json");

        let json = GenesisJson::load(path).unwrap();
        let genesis = Genesis::try_from(json.clone()).unwrap();

        let json_again = GenesisJson::try_from(genesis.clone()).unwrap();
        let genesis_again = Genesis::try_from(json_again.clone()).unwrap();

        similar_asserts::assert_eq!(genesis, genesis_again);
    }

    #[test]
    fn default_genesis_try_from_json() {
        let json = r#"
        {
            "number": 0,
            "parentHash": "0x999",
            "timestamp": 5123512314,
            "stateRoot": "0x99",
            "sequencerAddress": "0x100",
            "gasPrices": {
                "ETH": 1111,
                "STRK": 2222
            },
            "feeToken": {
                "name": "ETHER",
                "symbol": "ETH",
                "decimals": 18
            },
            "universalDeployer": {},
            "accounts": {
                "0x66efb28ac62686966ae85095ff3a772e014e7fbf56d4c5f6fac5606d4dde23a": {
                    "publicKey": "0x1",
                    "balance": "0xD3C21BCECCEDA1000000"
                }
            },
            "contracts": {},
            "classes": []
        }
        "#;

        let genesis_json: GenesisJson = GenesisJson::from_str(json).unwrap();
        let actual_genesis = Genesis::try_from(genesis_json).unwrap();

        let mut classes = BTreeMap::new();
        classes.insert(DEFAULT_ACCOUNT_CLASS_HASH, DEFAULT_ACCOUNT_CLASS.clone().into());

        let allocations = BTreeMap::from([(
            address!("0x66efb28ac62686966ae85095ff3a772e014e7fbf56d4c5f6fac5606d4dde23a"),
            GenesisAllocation::Account(GenesisAccountAlloc::Account(GenesisAccount {
                public_key: felt!("0x1"),
                balance: Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap()),
                class_hash: DEFAULT_ACCOUNT_CLASS_HASH,
                nonce: None,
                storage: None,
                salt: GenesisAccount::DEFAULT_SALT,
            })),
        )]);

        let expected_genesis = Genesis {
            classes,
            allocations,
            number: 0,
            timestamp: 5123512314u64,
            state_root: felt!("0x99"),
            parent_hash: felt!("0x999"),
            sequencer_address: address!("0x100"),
            gas_prices: unsafe { GasPrices::new_unchecked(1111, 2222) },
        };

        assert_eq!(actual_genesis.allocations.len(), expected_genesis.allocations.len());

        for (address, alloc) in actual_genesis.allocations {
            let expected_alloc = expected_genesis.allocations.get(&address).unwrap();
            assert_eq!(alloc, *expected_alloc);
        }

        // assert that the list of classes is the same
        assert_eq!(actual_genesis.classes.len(), expected_genesis.classes.len());

        for (hash, class) in actual_genesis.classes {
            let expected_class = expected_genesis.classes.get(&hash).unwrap();
            assert_eq!(&class, expected_class);
        }
    }

    #[test]
    fn genesis_from_json_with_unresolved_paths() {
        let file = File::open("./src/genesis/test-genesis.json").unwrap();
        let json: GenesisJson = serde_json::from_reader(file).unwrap();
        assert!(Genesis::try_from(json)
            .unwrap_err()
            .to_string()
            .contains("Unresolved class artifact path"));
    }

    #[test]
    fn encode_decode_genesis_file_to_base64() {
        let path = PathBuf::from("./src/genesis/test-genesis.json");

        let genesis = GenesisJson::load(path).unwrap();
        let genesis_clone = genesis.clone();

        let encoded = to_base64(genesis_clone).unwrap();
        let decoded = from_base64(encoded.as_slice()).unwrap();

        similar_asserts::assert_eq!(genesis, decoded);
    }

    #[test]
    fn account_with_unknown_class() {
        let name = "MyClass";

        let account = GenesisAccountJson {
            salt: None,
            nonce: None,
            storage: None,
            balance: None,
            private_key: None,
            public_key: Default::default(),
            class: Some(ClassNameOrHash::Name(name.to_string())),
        };

        let mut json = GenesisJson::default();
        json.accounts.insert(felt!("1").into(), account);

        let res = Genesis::try_from(json);
        assert!(res.unwrap_err().to_string().contains(&format!("Class name '{name}' not found")))
    }

    #[test]
    fn classes_with_duplicate_names() {
        let name = "MyClass";

        let json = GenesisJson::load("./src/genesis/test-genesis-with-duplicate-name.json")
            .expect("failed to load genesis file");

        let res = Genesis::try_from(json);
        assert!(res
            .unwrap_err()
            .to_string()
            .contains(&format!("Class name '{name}' already exists")))
    }
}
