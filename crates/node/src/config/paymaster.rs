use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymasterConfig {
    /// The root URL for the Cartridge API.
    pub cartridge_api_url: Url,
}
