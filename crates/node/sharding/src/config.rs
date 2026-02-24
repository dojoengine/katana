use std::sync::Arc;
use std::time::Duration;

use katana_chain_spec::ChainSpec;
use katana_node_config::dev::DevConfig;
use katana_node_config::execution::ExecutionConfig;
use katana_node_config::metrics::MetricsConfig;
use katana_node_config::rpc::RpcConfig;
use url::Url;

/// Default number of shard workers (matches available CPU cores).
pub fn default_worker_count() -> usize {
    std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1)
}

/// Default time quantum for worker preemption.
pub const DEFAULT_TIME_QUANTUM: Duration = Duration::from_millis(100);

/// Default poll interval for the block context listener.
pub const DEFAULT_BLOCK_POLL_INTERVAL: Duration = Duration::from_secs(12);

/// Configuration for the shard node.
#[derive(Debug, Clone)]
pub struct ShardNodeConfig {
    /// The chain specification.
    pub chain: Arc<ChainSpec>,

    /// RPC server configuration.
    pub rpc: RpcConfig,

    /// Execution configuration.
    pub execution: ExecutionConfig,

    /// Development configuration.
    pub dev: DevConfig,

    /// Number of shard workers.
    pub worker_count: usize,

    /// Time quantum for worker preemption (how long a worker processes a shard before yielding).
    pub time_quantum: Duration,

    /// RPC URL of the base/settlement chain for the block context listener.
    pub base_chain_url: Url,

    /// Polling interval for fetching new blocks from the base chain.
    pub block_poll_interval: Duration,

    /// Metrics configuration.
    pub metrics: Option<MetricsConfig>,
}

impl ShardNodeConfig {
    pub fn new(chain: Arc<ChainSpec>, base_chain_url: Url) -> Self {
        Self {
            chain,
            rpc: RpcConfig::default(),
            execution: ExecutionConfig::default(),
            dev: DevConfig::default(),
            worker_count: default_worker_count(),
            time_quantum: DEFAULT_TIME_QUANTUM,
            base_chain_url,
            block_poll_interval: DEFAULT_BLOCK_POLL_INTERVAL,
            metrics: None,
        }
    }
}
