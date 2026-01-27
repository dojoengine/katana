use std::path::PathBuf;

use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceMode {
    Disabled,
    Sidecar,
    External,
}

impl ServiceMode {
    pub fn is_enabled(self) -> bool {
        !matches!(self, ServiceMode::Disabled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrfKeySource {
    Prefunded,
    Sequencer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymasterConfig {
    pub mode: ServiceMode,
    pub url: Url,
    pub api_key: Option<String>,
    pub price_api_key: Option<String>,
    pub prefunded_index: u16,
    pub sidecar_port: u16,
    pub sidecar_bin: Option<PathBuf>,
    #[cfg(feature = "cartridge")]
    pub cartridge_api_url: Option<Url>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VrfConfig {
    pub mode: ServiceMode,
    pub url: Url,
    pub key_source: VrfKeySource,
    pub prefunded_index: u16,
    pub sidecar_port: u16,
    pub sidecar_bin: Option<PathBuf>,
}
