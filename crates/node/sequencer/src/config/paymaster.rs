#[cfg(feature = "cartridge")]
use katana_primitives::{ContractAddress, Felt};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymasterConfig {
    /// Paymaster service URL.
    pub url: Url,
    /// Optional API key for authentication.
    pub api_key: Option<String>,

    #[cfg(feature = "cartridge")]
    pub cartridge_api: Option<CartridgeApiConfig>,
}

/// Configuration for connecting to a Cartridge paymaster service.
///
/// The node treats the paymaster as an external service - it simply connects to the
/// provided URL. Whether the service is managed externally or as a sidecar process
/// is a concern of the CLI layer, not the node.
///
/// This config is specific to the Cartridge integration and requires explicit
/// account credentials for the paymaster account.
#[cfg(feature = "cartridge")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartridgeApiConfig {
    /// Cartridge API URL.
    pub cartridge_api_url: Url,
    /// The paymaster account address. (used for deploying controller)
    pub controller_deployer_address: ContractAddress,
    /// The paymaster account private key. (used for deploying controller)
    pub controller_deployer_private_key: Felt,

    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfConfig>,
}

/// Configuration for connecting to a VRF service.
///
/// The node treats the VRF as an external service - it simply connects to the
/// provided URL. Whether the service is managed externally or as a sidecar process
/// is not a concern of the [`Node`](crate::Node).
#[cfg(feature = "vrf")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VrfConfig {
    /// The VRF service URL.
    pub url: Url,
    /// The address of the VRF account contract.
    pub vrf_account: ContractAddress,
}
