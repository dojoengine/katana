use std::path::{Path, PathBuf};

use anyhow::Result;
use katana_messaging::MessagingConfig;
use serde::{Deserialize, Serialize};

use crate::options::*;

/// Node arguments configuration file.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct NodeArgsConfig {
    pub no_mining: Option<bool>,
    pub block_time: Option<u64>,
    pub block_cairo_steps_limit: Option<u64>,
    pub db_dir: Option<PathBuf>,
    pub messaging: Option<MessagingConfig>,
    pub logging: Option<LoggingOptions>,
    pub starknet: Option<StarknetOptions>,
    pub gpo: Option<GasPriceOracleOptions>,
    pub forking: Option<ForkingOptions>,
    #[serde(rename = "dev")]
    pub development: Option<DevOptions>,
    #[cfg(feature = "server")]
    pub server: Option<ServerOptions>,
    #[cfg(feature = "server")]
    pub metrics: Option<MetricsOptions>,
    #[cfg(feature = "cartridge")]
    pub cartridge: Option<CartridgeOptions>,
}

impl NodeArgsConfig {
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&file)?)
    }
}