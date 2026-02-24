use std::sync::Arc;

// Re-export shared config types
pub use katana_node_config::{db, metrics, rpc};

pub mod fork;
pub mod gateway;
pub mod sequencing;

#[cfg(feature = "paymaster")]
pub mod paymaster;
#[cfg(feature = "tee")]
pub mod tee;

#[cfg(feature = "grpc")]
pub mod grpc;

use db::DbConfig;
use fork::ForkingConfig;
use gateway::GatewayConfig;
#[cfg(feature = "grpc")]
use grpc::GrpcConfig;
use katana_chain_spec::ChainSpec;
use katana_messaging::MessagingConfig;
pub use katana_node_config::dev::DevConfig;
pub use katana_node_config::execution::ExecutionConfig;
use metrics::MetricsConfig;
use rpc::RpcConfig;
use sequencing::SequencingConfig;

/// Node configurations.
///
/// List of all possible options that can be used to configure a node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// The chain specification.
    pub chain: Arc<ChainSpec>,

    /// Database options.
    pub db: DbConfig,

    /// Forking options.
    pub forking: Option<ForkingConfig>,

    /// Rpc options.
    pub rpc: RpcConfig,

    /// Feeder gateway options.
    pub gateway: Option<GatewayConfig>,

    /// Metrics options.
    pub metrics: Option<MetricsConfig>,

    /// Execution options.
    pub execution: ExecutionConfig,

    /// Messaging options.
    pub messaging: Option<MessagingConfig>,

    /// Sequencing options.
    pub sequencing: SequencingConfig,

    /// Development options.
    pub dev: DevConfig,

    /// Cartridge paymaster options.
    #[cfg(feature = "paymaster")]
    pub paymaster: Option<paymaster::PaymasterConfig>,

    /// TEE attestation options.
    #[cfg(feature = "tee")]
    pub tee: Option<tee::TeeConfig>,

    /// gRPC options.
    #[cfg(feature = "grpc")]
    pub grpc: Option<GrpcConfig>,
}
