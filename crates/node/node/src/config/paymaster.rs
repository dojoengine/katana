use url::Url;

#[derive(Debug, Clone)]
pub struct PaymasterConfig {
    /// The root URL for the Cartridge API.
    pub cartridge_api_url: Url,
}
