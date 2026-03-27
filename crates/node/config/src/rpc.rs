use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use http::HeaderValue;
use serde::{Deserialize, Serialize};

pub const DEFAULT_RPC_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const DEFAULT_RPC_PORT: u16 = 5050;

/// Default maximmum page size for the `starknet_getEvents` RPC method.
pub const DEFAULT_RPC_MAX_EVENT_PAGE_SIZE: u64 = 1024;
/// Default maximmum number of keys for the `starknet_getStorageProof` RPC method.
pub const DEFAULT_RPC_MAX_PROOF_KEYS: u64 = 100;
/// Default maximum gas for the `starknet_call` RPC method.
pub const DEFAULT_RPC_MAX_CALL_GAS: u64 = 1_000_000_000;

/// Supported Starknet JSON-RPC spec versions.
#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    strum_macros::EnumString,
    strum_macros::Display,
    Serialize,
    Deserialize,
)]
pub enum StarknetApiVersion {
    #[strum(serialize = "v0.9", serialize = "v0_9", serialize = "0.9")]
    #[serde(rename = "v0.9")]
    V0_9,

    #[strum(serialize = "v0.10", serialize = "v0_10", serialize = "0.10")]
    #[serde(rename = "v0.10")]
    V0_10,
}

impl StarknetApiVersion {
    /// Returns the URL path segment for this version (e.g., `/v0_9`).
    pub fn path_segment(&self) -> &'static str {
        match self {
            Self::V0_9 => "/v0_9",
            Self::V0_10 => "/v0_10",
        }
    }
}

/// List of RPC modules supported by Katana.
#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    strum_macros::EnumString,
    strum_macros::Display,
    Serialize,
    Deserialize,
)]
#[strum(ascii_case_insensitive)]
pub enum RpcModuleKind {
    Starknet,
    Dev,
    Katana,
    TxPool,
    #[cfg(feature = "cartridge")]
    Cartridge,
    #[cfg(feature = "tee")]
    Tee,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid starknet api version: {0}")]
pub struct InvalidStarknetApiVersionError(String);

/// A set of [`StarknetApiVersion`]s to expose.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct StarknetApiVersionsList(HashSet<StarknetApiVersion>);

impl StarknetApiVersionsList {
    /// Creates an empty list.
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    /// Creates a list with all supported versions.
    pub fn all() -> Self {
        Self(HashSet::from([StarknetApiVersion::V0_9, StarknetApiVersion::V0_10]))
    }

    pub fn add(&mut self, version: StarknetApiVersion) {
        self.0.insert(version);
    }

    pub fn contains(&self, version: &StarknetApiVersion) -> bool {
        self.0.contains(version)
    }

    pub fn iter(&self) -> impl Iterator<Item = &StarknetApiVersion> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Used as the value parser for `clap`.
    pub fn parse(value: &str) -> Result<Self, InvalidStarknetApiVersionError> {
        if value.is_empty() {
            return Ok(Self::new());
        }

        let mut versions = HashSet::new();
        for v in value.split(',') {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                continue;
            }

            let version = trimmed
                .parse::<StarknetApiVersion>()
                .map_err(|_| InvalidStarknetApiVersionError(trimmed.to_string()))?;

            versions.insert(version);
        }

        Ok(Self(versions))
    }
}

impl Default for StarknetApiVersionsList {
    fn default() -> Self {
        Self::all()
    }
}

/// Starknet API-specific configuration within the RPC server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StarknetApiConfig {
    /// Which spec versions to expose at `/rpc/<version>`.
    pub versions: StarknetApiVersionsList,

    /// The default version served at the root path `/`.
    pub default_version: StarknetApiVersion,

    /// Maximum page size for `starknet_getEvents`.
    pub max_event_page_size: Option<u64>,

    /// Maximum number of keys for `starknet_getStorageProof`.
    pub max_proof_keys: Option<u64>,

    /// Maximum gas for `starknet_call`.
    pub max_call_gas: Option<u64>,

    /// Maximum concurrent `starknet_estimateFee` requests.
    pub max_concurrent_estimate_fee_requests: Option<u32>,
}

impl Default for StarknetApiConfig {
    fn default() -> Self {
        Self {
            versions: StarknetApiVersionsList::default(),
            default_version: StarknetApiVersion::V0_9,
            max_event_page_size: Some(DEFAULT_RPC_MAX_EVENT_PAGE_SIZE),
            max_proof_keys: Some(DEFAULT_RPC_MAX_PROOF_KEYS),
            max_call_gas: Some(DEFAULT_RPC_MAX_CALL_GAS),
            max_concurrent_estimate_fee_requests: None,
        }
    }
}

/// Configuration for the RPC server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcConfig {
    pub addr: IpAddr,
    pub port: u16,
    #[cfg(feature = "explorer")]
    pub explorer: bool,
    pub apis: RpcModulesList,
    pub cors_origins: Vec<HeaderValue>,
    pub max_connections: Option<u32>,
    pub max_request_body_size: Option<u32>,
    pub max_response_body_size: Option<u32>,
    pub timeout: Option<Duration>,
    /// Starknet API-specific configuration.
    pub starknet: StarknetApiConfig,
}

impl RpcConfig {
    /// Returns the [`SocketAddr`] for the RPC server.
    pub const fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            #[cfg(feature = "explorer")]
            explorer: false,
            cors_origins: Vec::new(),
            addr: DEFAULT_RPC_ADDR,
            port: DEFAULT_RPC_PORT,
            max_connections: None,
            max_request_body_size: None,
            max_response_body_size: None,
            timeout: None,
            apis: RpcModulesList::default(),
            starknet: StarknetApiConfig::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid module: {0}")]
pub struct InvalidRpcModuleError(String);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct RpcModulesList(HashSet<RpcModuleKind>);

impl RpcModulesList {
    /// Creates an empty modules list.
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    /// Creates a list with all the possible modules.
    pub fn all() -> Self {
        Self(HashSet::from([
            RpcModuleKind::Starknet,
            RpcModuleKind::Katana,
            RpcModuleKind::Dev,
            RpcModuleKind::TxPool,
            #[cfg(feature = "cartridge")]
            RpcModuleKind::Cartridge,
            #[cfg(feature = "tee")]
            RpcModuleKind::Tee,
        ]))
    }

    /// Adds a `module` to the list.
    pub fn add(&mut self, module: RpcModuleKind) {
        self.0.insert(module);
    }

    /// Returns `true` if the list contains the specified `module`.
    pub fn contains(&self, module: &RpcModuleKind) -> bool {
        self.0.contains(module)
    }

    /// Returns the number of modules in the list.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the list contains no modules.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Used as the value parser for `clap`.
    pub fn parse(value: &str) -> Result<Self, InvalidRpcModuleError> {
        if value.is_empty() {
            return Ok(Self::new());
        }

        let mut modules = HashSet::new();
        for module_name in value.split(',') {
            let trimmed_module_name = module_name.trim();

            if trimmed_module_name.is_empty() {
                continue;
            }

            let module: RpcModuleKind = trimmed_module_name
                .parse()
                .map_err(|_| InvalidRpcModuleError(trimmed_module_name.to_string()))?;

            modules.insert(module);
        }

        Ok(Self(modules))
    }
}

impl Default for RpcModulesList {
    fn default() -> Self {
        Self(HashSet::from([RpcModuleKind::Starknet]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty() {
        let list = RpcModulesList::parse("").unwrap();
        assert_eq!(list, RpcModulesList::new());
    }

    #[test]
    fn test_parse_single() {
        let list = RpcModulesList::parse("dev").unwrap();
        assert!(list.contains(&RpcModuleKind::Dev));
    }

    #[test]
    fn test_parse_multiple() {
        let list = RpcModulesList::parse("starknet,dev").unwrap();
        assert!(list.contains(&RpcModuleKind::Starknet));
        assert!(list.contains(&RpcModuleKind::Dev));
    }

    #[test]
    fn test_parse_with_spaces() {
        let list = RpcModulesList::parse(" dev ,   ").unwrap();
        assert!(list.contains(&RpcModuleKind::Dev));
    }

    #[test]
    fn test_parse_duplicates() {
        let list = RpcModulesList::parse("dev,dev,starknet").unwrap();
        let mut expected = RpcModulesList::new();
        expected.add(RpcModuleKind::Starknet);
        expected.add(RpcModuleKind::Dev);
        assert_eq!(list, expected);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(RpcModulesList::parse("invalid").is_err());
    }
}
