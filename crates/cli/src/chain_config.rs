//! On-disk chain configuration.
//!
//! A chain config directory holds two files:
//! - `config.toml` — the [`ChainSpec`] (any kind) plus an optional [`SettlementConfig`].
//! - `genesis.json` — the genesis state.
//!
//! The format is kind-agnostic: every `ChainSpec` variant serializes the same
//! common fields (`id`, `fee-contract`), distinguished only by a `kind` tag.
//! Settlement is a separate, optional section — it is not part of the chain
//! spec.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};

use katana_chain_spec::{dev, full_node, rollup, ChainSpec, FeeContracts, SettlementConfig};
use katana_genesis::json::GenesisJson;
use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::ContractAddress;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("OS not supported")]
    UnsupportedOS,

    #[error("No local config directory found for chain `{id}`")]
    LocalConfigDirectoryNotFound { id: String },

    #[error("Chain config path must be a directory")]
    MustBeADirectory,

    #[error("Failed to read config file: {0}")]
    ConfigReadError(#[from] toml::ser::Error),

    #[error("Failed to write config file: {0}")]
    ConfigWriteError(#[from] toml::de::Error),

    #[error("Missing chain configuration file")]
    MissingConfigFile,

    #[error("Missing genesis file")]
    MissingGenesisFile,

    #[error(transparent)]
    IO(#[from] std::io::Error),

    #[error(transparent)]
    GenesisJson(#[from] katana_genesis::json::GenesisJsonError),
}

/// Read the [`ChainSpec`] + optional [`SettlementConfig`] of the given `id` from the local config
/// directory.
pub fn read_local(id: &ChainId) -> Result<(ChainSpec, Option<SettlementConfig>), Error> {
    read(&ChainConfigDir::open_local(id)?)
}

/// Write the given [`ChainSpec`] + optional [`SettlementConfig`] at the local config directory
/// based on the chain id.
pub fn write_local(
    chain_spec: &ChainSpec,
    settlement: Option<&SettlementConfig>,
) -> Result<(), Error> {
    write(&ChainConfigDir::create_local(&chain_spec.id())?, chain_spec, settlement)
}

/// List all of the available chain configurations.
///
/// This will list only the configurations that are stored in the default local directory. See
/// [`local_dir`].
pub fn list() -> Result<Vec<ChainId>, Error> {
    list_at(local_dir()?)
}

pub fn read(dir: &ChainConfigDir) -> Result<(ChainSpec, Option<SettlementConfig>), Error> {
    let config_path = dir.config_path();
    let genesis_path = dir.genesis_path();

    if !config_path.exists() {
        return Err(Error::MissingConfigFile);
    }

    if !genesis_path.exists() {
        return Err(Error::MissingGenesisFile);
    }

    let config: ChainConfigFile = {
        let content = fs::read_to_string(config_path)?;
        toml::from_str(&content)?
    };

    let genesis: Genesis = {
        let file = BufReader::new(File::open(genesis_path)?);
        let json: GenesisJson = serde_json::from_reader(file).map_err(io::Error::from)?;
        Genesis::try_from(json)?
    };

    let id = config.id;
    let fee_contracts = config.fee_contract.into();
    let chain_spec = match config.kind {
        ChainKind::Dev => ChainSpec::Dev(dev::ChainSpec { id, genesis, fee_contracts }),
        ChainKind::Rollup => ChainSpec::Rollup(rollup::ChainSpec { id, genesis, fee_contracts }),
        ChainKind::FullNode => {
            ChainSpec::FullNode(full_node::ChainSpec { id, genesis, fee_contracts })
        }
    };

    Ok((chain_spec, config.settlement))
}

pub fn write(
    dir: &ChainConfigDir,
    chain_spec: &ChainSpec,
    settlement: Option<&SettlementConfig>,
) -> Result<(), Error> {
    {
        let cfg = ChainConfigFile {
            kind: ChainKind::of(chain_spec),
            id: chain_spec.id(),
            fee_contract: chain_spec.fee_contracts().clone().into(),
            settlement: settlement.cloned(),
        };

        let content = toml::to_string_pretty(&cfg)?;
        std::fs::write(dir.config_path(), &content)?;
    }

    {
        let genesis_json = GenesisJson::try_from(chain_spec.genesis().clone())?;
        let file = BufWriter::new(File::create(dir.genesis_path())?);
        serde_json::to_writer_pretty(file, &genesis_json).map_err(io::Error::from)?;
    }

    Ok(())
}

fn list_at<P: AsRef<Path>>(dir: P) -> Result<Vec<ChainId>, Error> {
    let mut chains = Vec::new();
    let dir = dir.as_ref();

    if dir.exists() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;

            // Ignore entry that is:-
            //
            // - not a directory
            // - name can't be parse as chain id
            // - config file is not found inside the directory
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Ok(chain_id) = ChainId::parse(name) {
                        let cs = LocalChainConfigDir::open_at(dir, &chain_id).expect("must exist");
                        if cs.config_path().exists() {
                            chains.push(chain_id);
                        }
                    }
                }
            }
        }
    }

    Ok(chains)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileFeeContract {
    pub strk: ContractAddress,
}

impl From<FileFeeContract> for FeeContracts {
    fn from(fee_contract: FileFeeContract) -> Self {
        Self { strk: fee_contract.strk, eth: fee_contract.strk }
    }
}

impl From<FeeContracts> for FileFeeContract {
    fn from(fee_contracts: FeeContracts) -> Self {
        Self { strk: fee_contracts.strk }
    }
}

/// Which [`ChainSpec`] variant a config file describes. Distinguishes the
/// otherwise field-identical specs (they differ only in genesis construction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ChainKind {
    Dev,
    Rollup,
    FullNode,
}

impl ChainKind {
    fn of(chain_spec: &ChainSpec) -> Self {
        match chain_spec {
            ChainSpec::Dev(_) => Self::Dev,
            ChainSpec::Rollup(_) => Self::Rollup,
            ChainSpec::FullNode(_) => Self::FullNode,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ChainConfigFile {
    kind: ChainKind,
    id: ChainId,
    fee_contract: FileFeeContract,
    /// Settlement is optional and orthogonal to the chain spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settlement: Option<SettlementConfig>,
}

/// The local directory name where the chain configuration files are stored.
const KATANA_LOCAL_DIR: &str = "katana";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChainConfigDir {
    Absolute(PathBuf),
    Local(LocalChainConfigDir),
}

impl ChainConfigDir {
    pub fn create_local(id: &ChainId) -> Result<Self, Error> {
        Ok(Self::Local(LocalChainConfigDir::create(id)?))
    }

    pub fn open_local(id: &ChainId) -> Result<Self, Error> {
        Ok(Self::Local(LocalChainConfigDir::open(id)?))
    }

    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();

        if !path.exists() {
            std::fs::create_dir_all(path)?;
        }

        Ok(ChainConfigDir::Absolute(path.to_path_buf()))
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = fs::canonicalize(path)?;

        if !path.is_dir() {
            return Err(Error::MustBeADirectory);
        }

        Ok(Self::Absolute(path.to_path_buf()))
    }

    pub fn config_path(&self) -> PathBuf {
        match self {
            Self::Absolute(path) => path.join("config").with_extension("toml"),
            Self::Local(local) => local.config_path(),
        }
    }

    pub fn genesis_path(&self) -> PathBuf {
        match self {
            Self::Absolute(path) => path.join("genesis").with_extension("json"),
            Self::Local(local) => local.genesis_path(),
        }
    }
}

// > LOCAL_DIR/$chain_id/
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalChainConfigDir(PathBuf);

impl LocalChainConfigDir {
    /// Creates a new config directory for the given chain ID.
    ///
    /// The directory will be created at `$LOCAL_DIR/<id>`, where `$LOCAL_DIR` is the path returned
    /// by [`local_dir`].
    ///
    /// This will create the directory if it does not yet exist.
    pub fn create(id: &ChainId) -> Result<Self, Error> {
        Self::create_at(local_dir()?, id)
    }

    /// Opens an existing config directory for the given chain ID.
    ///
    /// The path of the directory is expected to be `$LOCAL_DIR/<id>`, where `$LOCAL_DIR` is the
    /// path returned by [`local_dir`].
    ///
    /// # Errors
    ///
    /// This function will return an error if no directory exists with the given chain ID.
    pub fn open(id: &ChainId) -> Result<Self, Error> {
        Self::open_at(local_dir()?, id)
    }

    /// Same like [`Self::create`] but at a specific base path instead of `$LOCAL_DIR`.
    pub fn create_at<P: AsRef<Path>>(base: P, id: &ChainId) -> Result<Self, Error> {
        let id = id.to_string();
        let path = base.as_ref().join(id);

        if !path.exists() {
            std::fs::create_dir_all(&path)?;
        }

        Ok(Self(path))
    }

    /// Same like [`Self::open`] but at a specific base path instead of `$LOCAL_DIR`.
    pub fn open_at<P: AsRef<Path>>(base: P, id: &ChainId) -> Result<Self, Error> {
        let id = id.to_string();
        let path = base.as_ref().join(&id);

        if !path.exists() {
            return Err(Error::LocalConfigDirectoryNotFound { id: id.clone() });
        }

        Ok(Self(path))
    }

    /// Get the path to the config file for this chain.
    ///
    /// > $LOCAL_DIR/$chain_id/config.toml
    pub fn config_path(&self) -> PathBuf {
        self.0.join("config").with_extension("toml")
    }

    /// Get the path to the genesis file for this chain.
    ///
    /// > $LOCAL_DIR/$chain_id/genesis.json
    pub fn genesis_path(&self) -> PathBuf {
        self.0.join("genesis").with_extension("json")
    }
}

/// ```text
/// | -------- | --------------------------------------------- |
/// | Platform | Path                                          |
/// | -------- | --------------------------------------------- |
/// | Linux    | `$XDG_CONFIG_HOME` or `$HOME`/.config/katana  |
/// | macOS    | `$HOME`/Library/Application Support/katana    |
/// | Windows  | `{FOLDERID_LocalAppData}`/katana              |
/// | -------- | --------------------------------------------- |
/// ```
pub fn local_dir() -> Result<PathBuf, Error> {
    Ok(dirs::config_local_dir().ok_or(Error::UnsupportedOS)?.join(KATANA_LOCAL_DIR))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::OnceLock;

    use katana_chain_spec::{
        rollup, ChainSpec, FeeContracts, SettlementConfig, SettlementLayer, SettlementProofKind,
    };
    use katana_genesis::Genesis;
    use katana_primitives::chain::ChainId;
    use katana_primitives::ContractAddress;
    use tempfile::TempDir;
    use url::Url;

    use super::{local_dir, ChainConfigDir, Error, LocalChainConfigDir, KATANA_LOCAL_DIR};

    static TEMPDIR: OnceLock<TempDir> = OnceLock::new();

    fn with_temp_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
        f(TEMPDIR.get_or_init(|| tempfile::TempDir::new().unwrap()).path())
    }

    /// Test version of [`super::read`].
    fn read(id: &ChainId) -> Result<(ChainSpec, Option<SettlementConfig>), Error> {
        with_temp_dir(|dir| {
            let dir = LocalChainConfigDir::open_at(dir, id)?;
            super::read(&ChainConfigDir::Local(dir))
        })
    }

    /// Test version of [`super::write`].
    fn write(chain_spec: &ChainSpec, settlement: Option<&SettlementConfig>) -> Result<(), Error> {
        with_temp_dir(|dir| {
            let dir = LocalChainConfigDir::create_at(dir, &chain_spec.id())?;
            super::write(&ChainConfigDir::Local(dir), chain_spec, settlement)
        })
    }

    impl LocalChainConfigDir {
        fn open_tmp(id: &ChainId) -> Result<Self, Error> {
            with_temp_dir(|dir| Self::open_at(dir, id))
        }

        fn create_tmp(id: &ChainId) -> Result<Self, Error> {
            with_temp_dir(|dir| Self::create_at(dir, id))
        }
    }

    fn chainspec() -> ChainSpec {
        ChainSpec::Rollup(rollup::ChainSpec {
            id: ChainId::default(),
            genesis: Genesis::default(),
            fee_contracts: FeeContracts {
                eth: ContractAddress::default(),
                strk: ContractAddress::default(),
            },
        })
    }

    fn starknet_layer() -> SettlementLayer {
        SettlementLayer::Starknet {
            block: 0,
            id: ChainId::default(),
            core_contract: ContractAddress::default(),
            rpc_url: Url::parse("http://localhost:5050").expect("valid url"),
            proof_kind: SettlementProofKind::Tee,
        }
    }

    #[test]
    fn read_write_chainspec_without_settlement() {
        let chain_spec = chainspec();
        let id = chain_spec.id();

        write(&chain_spec, None).unwrap();
        let (read_spec, settlement) = read(&id).unwrap();

        assert_eq!(chain_spec.id(), read_spec.id());
        assert_eq!(chain_spec.fee_contracts(), read_spec.fee_contracts());
        assert!(matches!(read_spec, ChainSpec::Rollup(_)));
        assert_eq!(settlement, None);
    }

    /// The settlement section round-trips through the TOML file, including the serde defaults for
    /// the runtime batching knobs.
    #[test]
    fn settlement_round_trip() {
        use katana_chain_spec::SettlementRuntime;
        use katana_primitives::felt;

        let mut chain_spec = chainspec();
        if let ChainSpec::Rollup(spec) = &mut chain_spec {
            spec.id = ChainId::parse("settlement_round_trip").unwrap();
        }

        let settlement = SettlementConfig {
            layer: starknet_layer(),
            runtime: Some(SettlementRuntime {
                account_address: ContractAddress::from(felt!("0x123")),
                account_private_key: felt!("0x456"),
                tee_registry: ContractAddress::from(felt!("0x789")),
                prover_key: Some("sp1_dummy".to_string()),
                batch_size: 3,
                idle_flush_secs: 7,
            }),
        };

        write(&chain_spec, Some(&settlement)).unwrap();
        let (_, read_settlement) = read(&chain_spec.id()).unwrap();

        assert_eq!(Some(settlement), read_settlement);
    }

    /// A settlement section with no `[settlement.runtime]` parses with `runtime: None`; a config
    /// with no `[settlement]` parses to `None`.
    #[test]
    fn settlement_runtime_optionality() {
        let base = r#"
kind = "rollup"

[id]
Id = "0x4b4154414e41"

[fee-contract]
strk = "0x0"
"#;

        let parsed: super::ChainConfigFile =
            toml::from_str(base).expect("parses without settlement");
        assert!(parsed.settlement.is_none());

        let with_layer = format!(
            "{base}\n[settlement.layer.starknet]\nrpc_url = \"http://localhost:5050/\"\n\
             core_contract = \"0x0\"\nblock = 0\nproof_kind = \"tee\"\n\
             [settlement.layer.starknet.id]\nId = \"0x4b4154414e41\"\n"
        );
        let parsed: super::ChainConfigFile =
            toml::from_str(&with_layer).expect("parses with layer, no runtime");
        let settlement = parsed.settlement.expect("settlement present");
        assert!(settlement.runtime.is_none());
        assert!(matches!(settlement.layer, SettlementLayer::Starknet { .. }));
    }

    #[test]
    fn test_chain_config_dir() {
        let chain_id = ChainId::parse("test").unwrap();

        // Test creation
        let config_dir = LocalChainConfigDir::create_tmp(&chain_id).unwrap();
        assert!(config_dir.0.exists());

        // Test opening existing dir
        let opened_dir = LocalChainConfigDir::open_tmp(&chain_id).unwrap();
        assert_eq!(config_dir.0, opened_dir.0);

        // Test opening non-existent dir
        let bad_id = ChainId::parse("nonexistent").unwrap();
        assert!(matches!(
            LocalChainConfigDir::open_tmp(&bad_id),
            Err(Error::LocalConfigDirectoryNotFound { .. })
        ));
    }

    #[test]
    fn test_local_dir() {
        let dir = local_dir().unwrap();
        assert!(dir.ends_with(KATANA_LOCAL_DIR));
    }

    #[test]
    fn test_config_paths() {
        let chain_id = ChainId::parse("test").unwrap();
        let config_dir = LocalChainConfigDir::create_tmp(&chain_id).unwrap();

        assert!(config_dir.config_path().ends_with("config.toml"));
        assert!(config_dir.genesis_path().ends_with("genesis.json"));
    }

    #[test]
    fn test_list_chain_specs() {
        let dir = tempfile::TempDir::new().unwrap().keep();

        let listed_chains = super::list_at(&dir).unwrap();
        assert_eq!(listed_chains.len(), 0, "Must be empty initially");

        // Create some dummy chain specs
        let mut chain_specs = Vec::new();
        for i in 1..=3 {
            let mut spec = chainspec();
            // update the chain id to make they're unqiue
            if let ChainSpec::Rollup(s) = &mut spec {
                s.id = ChainId::parse(&format!("chain_{i}")).unwrap();
            }
            chain_specs.push(spec);
        }

        // Write them to disk
        for spec in &chain_specs {
            let id = spec.id();
            let dir = LocalChainConfigDir::create_at(&dir, &id).unwrap();
            super::write(&ChainConfigDir::Local(dir), spec, None).unwrap();
        }

        let listed_chains = super::list_at(&dir).unwrap();
        assert_eq!(listed_chains.len(), chain_specs.len());
    }

    #[test]
    fn test_absolute_chain_config_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        // Test creating absolute dir
        let chain_dir = ChainConfigDir::create(path).unwrap();
        match &chain_dir {
            ChainConfigDir::Absolute(p) => assert_eq!(p, &path),
            _ => panic!("Expected Absolute variant"),
        }

        // Test opening existing absolute dir
        let opened_dir = ChainConfigDir::open(path).unwrap();
        match opened_dir {
            ChainConfigDir::Absolute(p) => assert_eq!(p, fs::canonicalize(path).unwrap()),
            _ => panic!("Expected Absolute variant"),
        }

        // Test error on non-existent dir
        let bad_path = path.join("nonexistent");
        assert!(matches!(ChainConfigDir::open(&bad_path), Err(Error::IO(..))));
    }
}
