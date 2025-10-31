use std::sync::Arc;

use katana_chain_spec::ChainSpec;

use crate::config::db::DbConfig;
use crate::config::fork::ForkingConfig;
use crate::config::metrics::MetricsConfig;
#[cfg(feature = "cartridge")]
use crate::config::paymaster;
use crate::config::rpc::RpcConfig;

/// Node configurations.
///
/// List of all possible options that can be used to configure a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The chain specification.
    pub chain: Arc<ChainSpec>,

    /// Database options.
    pub db: DbConfig,

    /// Forking options.
    pub forking: ForkingConfig,

    /// Rpc options.
    pub rpc: RpcConfig,

    /// Metrics options.
    pub metrics: Option<MetricsConfig>,
}
