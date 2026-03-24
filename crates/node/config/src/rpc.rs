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
    pub max_concurrent_estimate_fee_requests: Option<u32>,
    pub max_request_body_size: Option<u32>,
    pub max_response_body_size: Option<u32>,
    pub timeout: Option<Duration>,
    pub max_proof_keys: Option<u64>,
    pub max_event_page_size: Option<u64>,
    pub max_call_gas: Option<u64>,
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
            max_concurrent_estimate_fee_requests: None,
            max_request_body_size: None,
            max_response_body_size: None,
            timeout: None,
            apis: RpcModulesList::default(),
            max_event_page_size: Some(DEFAULT_RPC_MAX_EVENT_PAGE_SIZE),
            max_proof_keys: Some(DEFAULT_RPC_MAX_PROOF_KEYS),
            max_call_gas: Some(DEFAULT_RPC_MAX_CALL_GAS),
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
