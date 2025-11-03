use std::sync::Arc;

use anyhow::Result;
pub use clap::Parser;
use katana_chain_spec::ChainSpec;
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

use crate::options::*;

pub(crate) const LOG_TARGET: &str = "katana::cli::optimistic";

#[derive(Parser, Debug, Serialize, Deserialize, Clone, PartialEq)]
#[command(next_help_heading = "Optimistic node options")]
pub struct OptimisticNodeArgs {
    /// Don't print anything on startup.
    #[arg(long)]
    #[serde(default)]
    pub silent: bool,

    /// The Starknet RPC provider to fork from.
    #[arg(long, value_name = "URL", alias = "rpc-url")]
    #[arg(help = "The Starknet RPC provider to fork from.")]
    pub fork_provider_url: Url,

    #[command(flatten)]
    #[serde(default)]
    pub logging: LoggingOptions,

    #[command(flatten)]
    #[serde(default)]
    pub tracer: TracerOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    #[serde(default)]
    pub metrics: MetricsOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    #[serde(default)]
    pub server: ServerOptions,
}

impl OptimisticNodeArgs {
    pub async fn execute(&self) -> Result<()> {
        let config = self.config()?;

        #[cfg(feature = "server")]
        let rpc_addr = config.rpc.socket_addr();

        if !self.silent {
            info!(target: LOG_TARGET, "Starting optimistic node...");
        }

        let node = katana_node::optimistic::Node::build(config).await?;
        let _handle = node.launch().await?;

        #[cfg(feature = "server")]
        {
            info!(target: LOG_TARGET, %rpc_addr, "JSON-RPC server started.");
        }

        // Wait indefinitely
        tokio::signal::ctrl_c().await?;

        Ok(())
    }

    fn config(&self) -> Result<katana_node::optimistic::config::Config> {
        let chain = self.chain_spec()?;
        let rpc = self.rpc_config()?;
        let forking = self.forking_config();
        let metrics = self.metrics_config()?;
        Ok(katana_node::optimistic::config::Config { chain, rpc, forking, metrics })
    }

    fn chain_spec(&self) -> Result<Arc<ChainSpec>> {
        // Always use dev chain spec for optimistic node
        Ok(Arc::new(ChainSpec::Dev(Default::default())))
    }

    fn forking_config(&self) -> katana_node::optimistic::config::ForkingConfig {
        use katana_node::optimistic::config::ForkingConfig;
        ForkingConfig { url: self.fork_provider_url.clone(), block: None }
    }

    fn rpc_config(&self) -> Result<katana_node::optimistic::config::RpcConfig> {
        use katana_node::optimistic::config::{RpcConfig, RpcModuleKind, RpcModulesList};
        #[cfg(feature = "server")]
        {
            let mut apis = RpcModulesList::new();
            apis.add(RpcModuleKind::Starknet);

            Ok(RpcConfig {
                addr: self.server.http_addr,
                port: self.server.http_port,
                apis,
                max_connections: self.server.max_connections,
                cors_origins: self.server.http_cors_origins.clone(),
                ..Default::default()
            })
        }

        #[cfg(not(feature = "server"))]
        Ok(RpcConfig::default())
    }

    fn metrics_config(&self) -> Result<Option<katana_node::optimistic::config::MetricsConfig>> {
        use katana_node::optimistic::config::MetricsConfig;
        #[cfg(feature = "server")]
        {
            if self.metrics.metrics {
                Ok(Some(MetricsConfig {
                    addr: self.metrics.metrics_addr,
                    port: self.metrics.metrics_port,
                }))
            } else {
                Ok(None)
            }
        }

        #[cfg(not(feature = "server"))]
        Ok(None)
    }
}
