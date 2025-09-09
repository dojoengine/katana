#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use katana_chain_spec::rollup::ChainConfigDir;
use serde::{Deserialize, Serialize};

pub mod file;
pub mod options;
pub mod utils;

pub use file::NodeArgsConfig;
pub use options::*;
use utils::parse_chain_config_dir;

#[derive(Parser, Debug, Serialize, Deserialize, Default, Clone)]
#[command(next_help_heading = "Node options")]
pub struct NodeArgs {
    /// Don't print anything on startup.
    #[arg(long)]
    pub silent: bool,

    /// Path to the chain configuration file.
    #[arg(long, hide = true)]
    #[arg(value_parser = parse_chain_config_dir)]
    pub chain: Option<ChainConfigDir>,

    /// Disable auto and interval mining, and mine on demand instead via an endpoint.
    #[arg(long)]
    #[arg(conflicts_with = "block_time")]
    pub no_mining: bool,

    /// Block time in milliseconds for interval mining.
    #[arg(short, long)]
    #[arg(value_name = "MILLISECONDS")]
    pub block_time: Option<u64>,

    #[arg(long = "sequencing.block-max-cairo-steps")]
    #[arg(value_name = "TOTAL")]
    pub block_cairo_steps_limit: Option<u64>,

    /// Directory path of the database to initialize from.
    ///
    /// The path must either be an empty directory or a directory which already contains a
    /// previously initialized Katana database.
    #[arg(long)]
    #[arg(value_name = "PATH")]
    pub db_dir: Option<PathBuf>,

    /// Configuration file
    #[arg(long)]
    pub config: Option<PathBuf>,

    // /// Configure the messaging with an other chain.
    // ///
    // /// Configure the messaging to allow Katana listening/sending messages on a
    // /// settlement chain that can be Ethereum or an other Starknet sequencer.
    // #[arg(long)]
    // #[arg(value_name = "PATH")]
    // #[arg(value_parser = katana_messaging::MessagingConfig::parse)]
    // #[arg(conflicts_with = "chain")]
    // pub messaging: Option<MessagingConfig>,
    // #[arg(long = "l1.provider", value_name = "URL", alias = "l1-provider")]
    // #[arg(help = "The Ethereum RPC provider to sample the gas prices from to enable the gas \
    //               price oracle.")]
    // pub l1_provider_url: Option<Url>,
    #[command(flatten)]
    pub messaging: MessagingOptions,

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

    #[command(flatten)]
    pub starknet: StarknetOptions,

    #[command(flatten)]
    pub gpo: GasPriceOracleOptions,

    #[command(flatten)]
    pub forking: ForkingOptions,

    #[command(flatten)]
    pub development: DevOptions,

    #[cfg(feature = "explorer")]
    #[command(flatten)]
    pub explorer: ExplorerOptions,

    #[cfg(feature = "cartridge")]
    #[command(flatten)]
    pub cartridge: CartridgeOptions,
}

impl NodeArgs {
    /// Parse the node config from the command line arguments and the config file,
    /// and merge them together prioritizing the command line arguments.
    pub fn with_config_file(&mut self) -> Result<()> {
        let config = if let Some(path) = &self.config {
            NodeArgsConfig::read(path)?
        } else {
            return Ok(());
        };

        // the CLI (self) takes precedence over the config file.
        // Currently, the merge is made at the top level of the commands.
        // We may add recursive merging in the future.

        if !self.no_mining {
            self.no_mining = config.no_mining.unwrap_or_default();
        }

        if self.block_time.is_none() {
            self.block_time = config.block_time;
        }

        if self.db_dir.is_none() {
            self.db_dir = config.db_dir;
        }

        if self.logging == LoggingOptions::default() {
            if let Some(logging) = config.logging {
                self.logging = logging;
            }
        }

        if self.messaging == MessagingOptions::default() {
            if let Some(messaging) = config.messaging {
                self.messaging = messaging;
            }
        }

        #[cfg(feature = "server")]
        {
            self.server.merge(config.server.as_ref());

            if self.metrics == MetricsOptions::default() {
                if let Some(metrics) = config.metrics {
                    self.metrics = metrics;
                }
            }
        }

        self.starknet.merge(config.starknet.as_ref());
        self.development.merge(config.development.as_ref());

        if self.gpo == GasPriceOracleOptions::default() {
            if let Some(gpo) = config.gpo {
                self.gpo = gpo;
            }
        }

        if self.forking == ForkingOptions::default() {
            if let Some(forking) = config.forking {
                self.forking = forking;
            }
        }

        #[cfg(feature = "cartridge")]
        {
            self.cartridge.merge(config.cartridge.as_ref());
        }

        Ok(())
    }
}
