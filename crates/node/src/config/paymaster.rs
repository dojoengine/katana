use url::Url;

use paymaster_rpc::context::Configuration as PaymasterConfiguration;

#[derive(Debug, Clone)]
pub struct PaymasterConfig {
    /// The root URL for the Cartridge API.
    pub cartridge_api_url: Url,
    /// Configuration for the paymaster services.
    pub paymaster: PaymasterConfiguration,
    /// Optional default API key forwarded to the paymaster endpoints.
    pub default_api_key: Option<String>,
}
