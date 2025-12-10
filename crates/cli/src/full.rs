use std::path::PathBuf;

use anyhow::{Context, Result};
pub use clap::Parser;
use katana_node::config::db::DbConfig;
use katana_node::config::metrics::MetricsConfig;
use katana_node::config::rpc::RpcConfig;
use katana_node::full;
use katana_node::full::Network;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::options::*;

pub(crate) const LOG_TARGET: &str = "katana::cli::full";

#[derive(Parser, Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
#[command(next_help_heading = "Full node options")]
pub struct FullNodeArgs {
    /// Don't print anything on startup.
    #[arg(long)]
    pub silent: bool,

    /// Directory path of the database to initialize from.
    ///
    /// The path must either be an empty directory or a directory which already contains a
    /// previously initialized Katana database.
    #[arg(long)]
    #[arg(value_name = "PATH")]
    pub db_dir: PathBuf,

    #[arg(long)]
    pub network: Network,

    /// Gateway API key for accessing the sequencer gateway.
    #[arg(long)]
    #[arg(value_name = "KEY")]
    pub gateway_api_key: Option<String>,

    #[command(flatten)]
    pub logging: LoggingOptions,

    #[command(flatten)]
    pub tracer: TracerOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub metrics: MetricsOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub server: ServerOptions,

    #[cfg(feature = "explorer")]
    #[command(flatten)]
    pub explorer: ExplorerOptions,

    #[command(flatten)]
    pub pruning: PruningOptions,
}

impl FullNodeArgs {
    pub async fn execute(&self) -> Result<()> {
        // Initialize logging with tracer
        let tracer_config = self.tracer_config();
        katana_tracing::init(self.logging.log_format, tracer_config).await?;
        self.start_node().await
    }

    async fn start_node(&self) -> Result<()> {
        // Build the node
        let config = self.config()?;
        let node = full::Node::build(config).context("failed to build full node")?;

        if !self.silent {
            info!(target: LOG_TARGET, "Starting full node");
        }

        // Launch the node
        let handle = node.launch().await.context("failed to launch full node")?;

        // Wait until an OS signal (ie SIGINT, SIGTERM) is received or the node is shutdown.
        tokio::select! {
            _ = katana_utils::wait_shutdown_signals() => {
                // Gracefully shutdown the node before exiting
                handle.stop().await?;
            },

            _ = handle.stopped() => { }
        }

        info!("Shutting down.");

        Ok(())
    }

    fn config(&self) -> Result<full::Config> {
        let db = self.db_config();
        let rpc = self.rpc_config()?;
        let metrics = self.metrics_config();
        let pruning = self.pruning_config();

        Ok(full::Config {
            db,
            rpc,
            metrics,
            pruning,
            network: self.network,
            gateway_api_key: self.gateway_api_key.clone(),
        })
    }

    fn pruning_config(&self) -> full::PruningConfig {
        use crate::options::PruningMode;

        // Translate CLI pruning mode to distance from tip
        let distance = match self.pruning.mode {
            PruningMode::Archive => None,
            PruningMode::Full(n) => Some(n),
        };

        full::PruningConfig { distance, interval: self.pruning.interval }
    }

    fn db_config(&self) -> DbConfig {
        DbConfig { dir: Some(self.db_dir.clone()) }
    }

    fn rpc_config(&self) -> Result<RpcConfig> {
        #[cfg(feature = "server")]
        {
            use std::time::Duration;

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
                explorer: self.explorer.explorer,
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
