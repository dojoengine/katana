use url::Url;

/// Key source for VRF secret key derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrfKeySource {
    Prefunded,
    Sequencer,
}

/// Configuration for connecting to a paymaster service.
///
/// The node treats the paymaster as an external service - it simply connects to the
/// provided URL. Whether the service is managed externally or as a sidecar process
/// is a concern of the CLI layer, not the node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymasterConfig {
    /// The paymaster service URL.
    pub url: Url,
    /// Optional API key for authentication.
    pub api_key: Option<String>,
    /// Prefunded account index used by the paymaster.
    pub prefunded_index: u16,
    /// Cartridge API URL (required for cartridge integration).
    #[cfg(feature = "cartridge")]
    pub cartridge_api_url: Option<Url>,
}

/// Configuration for connecting to a VRF service.
///
/// The node treats the VRF as an external service - it simply connects to the
/// provided URL. Whether the service is managed externally or as a sidecar process
/// is a concern of the CLI layer, not the node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VrfConfig {
    /// The VRF service URL.
    pub url: Url,
    /// Source for the VRF secret key.
    pub key_source: VrfKeySource,
    /// Prefunded account index used for VRF operations.
    pub prefunded_index: u16,
}
