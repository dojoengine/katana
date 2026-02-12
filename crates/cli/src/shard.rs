use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
pub use clap::Parser;
use katana_node::config::dev::DevConfig;
use katana_node::config::execution::ExecutionConfig;
use katana_node::config::metrics::MetricsConfig;
use katana_node::config::rpc::RpcConfig;
use katana_node::shard::config::{
    ShardNodeConfig, DEFAULT_BLOCK_POLL_INTERVAL, DEFAULT_TIME_QUANTUM,
};
use katana_node::shard::ShardNode;
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

use crate::options::*;

pub(crate) const LOG_TARGET: &str = "katana::cli::shard";

#[derive(Parser, Debug, Serialize, Deserialize, Clone, PartialEq)]
#[command(next_help_heading = "Shard node options")]
pub struct ShardNodeArgs {
    /// Don't print anything on startup.
    #[arg(long)]
    pub silent: bool,

    /// RPC URL of the base/settlement chain.
    ///
    /// The shard node polls this endpoint for block context updates (block number, timestamp,
    /// gas prices). This is typically the URL of a running Katana sequencer node or any
    /// Starknet-compatible RPC endpoint.
    #[arg(long = "base-chain-url", value_name = "URL")]
    pub base_chain_url: Url,

    /// Number of shard worker threads.
    ///
    /// Each worker runs on a dedicated OS thread and processes shards from the scheduler queue.
    /// Defaults to the number of available CPU cores.
    #[arg(long = "workers", value_name = "COUNT")]
    pub workers: Option<usize>,

    /// Time quantum in milliseconds for worker preemption.
    ///
    /// Controls how long a worker processes a single shard before yielding to allow other
    /// shards to be serviced.
    #[arg(long = "time-quantum", value_name = "MILLISECONDS")]
    #[arg(default_value_t = DEFAULT_TIME_QUANTUM.as_millis() as u64)]
    pub time_quantum_ms: u64,

    /// Base chain block poll interval in seconds.
    ///
    /// How frequently the shard node polls the base chain for new block context.
    #[arg(long = "block-poll-interval", value_name = "SECONDS")]
    #[arg(default_value_t = DEFAULT_BLOCK_POLL_INTERVAL.as_secs())]
    pub block_poll_interval_secs: u64,

    /// Disable charging fee when executing transactions.
    #[arg(long = "no-fee")]
    pub no_fee: bool,

    /// Disable account validation when executing transactions.
    ///
    /// Skipping the transaction sender's account validation function.
    #[arg(long = "no-account-validation")]
    pub no_account_validation: bool,

    #[command(flatten)]
    pub logging: LoggingOptions,

    #[command(flatten)]
    pub tracer: TracerOptions,

    #[command(flatten)]
    pub starknet: EnvironmentOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub server: ServerOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub metrics: MetricsOptions,
}

impl ShardNodeArgs {
    pub async fn execute(&self) -> Result<()> {
        let logging = katana_tracing::LoggingConfig {
            stdout_format: self.logging.stdout.stdout_format,
            stdout_color: self.logging.stdout.color,
            file_enabled: self.logging.file.enabled,
            file_format: self.logging.file.file_format,
            file_directory: self.logging.file.directory.clone(),
            file_max_files: self.logging.file.max_files,
        };

        katana_tracing::init(logging, self.tracer_config()).await?;

        self.start_node().await
    }

    async fn start_node(&self) -> Result<()> {
        let config = self.config()?;
        let node = ShardNode::build(config).context("failed to build shard node")?;

        if !self.silent {
            info!(target: LOG_TARGET, "Starting shard node");
        }

        let handle = node.launch().await.context("failed to launch shard node")?;

        // Wait until an OS signal (ie SIGINT, SIGTERM) is received or the node is shutdown.
        tokio::select! {
            _ = katana_utils::wait_shutdown_signals() => {
                handle.stop().await?;
            },

            _ = handle.stopped() => { }
        }

        info!("Shutting down.");

        Ok(())
    }

    fn config(&self) -> Result<ShardNodeConfig> {
        let chain = self.chain_spec();
        let rpc = self.rpc_config()?;
        let execution = self.execution_config();
        let dev = self.dev_config();
        let metrics = self.metrics_config();

        let worker_count =
            self.workers.unwrap_or_else(katana_node::shard::config::default_worker_count);

        Ok(ShardNodeConfig {
            chain,
            rpc,
            execution,
            dev,
            worker_count,
            time_quantum: Duration::from_millis(self.time_quantum_ms),
            base_chain_url: self.base_chain_url.clone(),
            block_poll_interval: Duration::from_secs(self.block_poll_interval_secs),
            metrics,
        })
    }

    fn chain_spec(&self) -> Arc<katana_chain_spec::ChainSpec> {
        let mut chain_spec = katana_chain_spec::dev::DEV_UNALLOCATED.clone();

        if let Some(id) = self.starknet.chain_id {
            chain_spec.id = id;
        }

        Arc::new(katana_chain_spec::ChainSpec::Dev(chain_spec))
    }

    fn rpc_config(&self) -> Result<RpcConfig> {
        #[cfg(feature = "server")]
        {
            let cors_origins = self.server.http_cors_origins.clone();

            Ok(RpcConfig {
                apis: Default::default(),
                port: self.server.http_port,
                addr: self.server.http_addr,
                max_connections: self.server.max_connections,
                max_concurrent_estimate_fee_requests: None,
                max_request_body_size: None,
                max_response_body_size: None,
                timeout: self.server.timeout.map(Duration::from_secs),
                cors_origins,
                #[cfg(feature = "explorer")]
                explorer: false,
                max_event_page_size: Some(self.server.max_event_page_size),
                max_proof_keys: Some(self.server.max_proof_keys),
                max_call_gas: Some(self.server.max_call_gas),
            })
        }

        #[cfg(not(feature = "server"))]
        {
            Ok(RpcConfig::default())
        }
    }

    fn execution_config(&self) -> ExecutionConfig {
        ExecutionConfig {
            invocation_max_steps: self.starknet.invoke_max_steps,
            validation_max_steps: self.starknet.validate_max_steps,
            #[cfg(feature = "native")]
            compile_native: self.starknet.compile_native,
            ..Default::default()
        }
    }

    fn dev_config(&self) -> DevConfig {
        DevConfig {
            fee: !self.no_fee,
            account_validation: !self.no_account_validation,
            fixed_gas_prices: None,
        }
    }

    fn metrics_config(&self) -> Option<MetricsConfig> {
        #[cfg(feature = "server")]
        if self.metrics.metrics {
            Some(MetricsConfig { addr: self.metrics.metrics_addr, port: self.metrics.metrics_port })
        } else {
            None
        }

        #[cfg(not(feature = "server"))]
        None
    }

    fn tracer_config(&self) -> Option<katana_tracing::TracerConfig> {
        self.tracer.config()
    }
}
