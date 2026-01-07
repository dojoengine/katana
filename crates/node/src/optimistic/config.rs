use std::sync::Arc;

use katana_chain_spec::ChainSpec;

pub use crate::config::db::DbConfig;
pub use crate::config::execution::ExecutionConfig;
pub use crate::config::fork::ForkingConfig;
pub use crate::config::metrics::MetricsConfig;
pub use crate::config::rpc::{RpcConfig, RpcModuleKind, RpcModulesList};

/// Node configurations.
///
/// List of all possible options that can be used to configure a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The chain specification.
    pub chain: Arc<ChainSpec>,

    /// Forking options.
    pub forking: ForkingConfig,

    /// Rpc options.
    pub rpc: RpcConfig,

    /// Metrics options.
    pub metrics: Option<MetricsConfig>,
}
