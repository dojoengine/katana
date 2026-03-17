use anyhow::{Context, Result};
pub use clap::Parser;
use katana_full_node::config::db::DbConfig;
use katana_full_node::config::gateway::GatewayConfig;
use katana_full_node::config::metrics::MetricsConfig;
use katana_full_node::config::rpc::RpcConfig;
use katana_full_node::config::trie::TrieConfig;
use katana_full_node::Network;
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

use crate::options::*;
use crate::utils::prompt_db_migration;

pub(crate) const LOG_TARGET: &str = "katana::cli::full";

#[derive(Parser, Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
#[command(next_help_heading = "Full node options")]
pub struct FullNodeArgs {
    /// Don't print anything on startup.
    #[arg(long)]
    pub silent: bool,

    #[arg(long)]
    pub network: Network,

    /// Gateway API key for accessing the sequencer gateway.
    #[arg(long)]
    #[arg(value_name = "KEY")]
    pub gateway_api_key: Option<String>,

    #[command(flatten)]
    pub db: DbOptions,

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

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub gateway: GatewayOptions,

    #[command(flatten)]
    pub trie: TrieOptions,

    #[command(flatten)]
    pub pruning: PruningOptions,

    /// The maximum block number to sync to. Once reached, the pipeline stops
    /// syncing but the node and RPC server remain running.
    #[arg(long = "sync.tip")]
    #[arg(value_name = "BLOCK_NUMBER")]
    pub max_sync_tip: Option<u64>,

    /// Custom feeder gateway base URL to sync from instead of the default
    /// network gateway. Useful for syncing from another katana node's
    /// feeder gateway.
    #[arg(long = "sync.gateway")]
    #[arg(value_name = "URL")]
    pub sync_gateway: Option<Url>,
}

impl FullNodeArgs {
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
        // Build the node
        let config = self.config()?;
        let node = katana_full_node::Node::build(config).context("failed to build full node")?;

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

    fn config(&self) -> Result<katana_full_node::Config> {
        let db = self.db_config()?;
        let rpc = self.rpc_config()?;
        let metrics = self.metrics_config();
        let pruning = self.pruning_config();
        let gateway = self.gateway_config();

        Ok(katana_full_node::Config {
            db,
            rpc,
            metrics,
            pruning,
            gateway,
            network: self.network,
            gateway_api_key: self.gateway_api_key.clone(),
            trie: TrieConfig { compute: !self.trie.disable },
            max_sync_tip: self.max_sync_tip,
            sync_gateway: self.sync_gateway.clone(),
        })
    }

    fn pruning_config(&self) -> katana_full_node::PruningConfig {
        use crate::options::PruningMode;

        // Translate CLI pruning mode to distance from tip
        let distance = match self.pruning.mode {
            PruningMode::Archive => None,
            PruningMode::Full(n) => Some(n),
        };

        katana_full_node::PruningConfig { distance }
    }

    fn gateway_config(&self) -> Option<GatewayConfig> {
        #[cfg(feature = "server")]
        if self.gateway.enable {
            return Some(GatewayConfig {
                addr: self.gateway.gateway_addr,
                port: self.gateway.gateway_port,
                timeout: Some(std::time::Duration::from_secs(self.gateway.gateway_timeout)),
            });
        }

        None
    }

    fn db_config(&self) -> Result<DbConfig> {
        let mut migrate = self.db.migrate;

        if !migrate {
            if let Some(ref path) = self.db.dir {
                if path.exists() {
                    migrate = prompt_db_migration(path)?;
                }
            }
        }

        Ok(DbConfig { dir: self.db.dir.clone(), migrate })
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
