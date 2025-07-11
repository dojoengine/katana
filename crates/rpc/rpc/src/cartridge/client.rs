use katana_primitives::{ContractAddress, Felt};
use serde::Deserialize;
use serde_json::json;
use url::Url;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("URL parsing error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("HTTP request error: {0}")]
    Request(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Client for interacting with the Cartridge service.
#[derive(Debug, Clone)]
pub struct CartridgeApiClient {
    url: Url,
    client: reqwest::Client,
}

impl CartridgeApiClient {
    /// Creates a new [`CartridgeApiClient`] with the given URL.
    pub fn new(url: Url) -> Self {
        Self { url, client: reqwest::Client::new() }
    }

    /// Fetch the calldata for the constructor of the given controller address.
    ///
    /// Returns `None` if the `address` is not associated with a Controller account.
    #[tracing::instrument(level = "trace", target = "cartridge", skip_all)]
    pub async fn get_account_calldata(
        &self,
        address: ContractAddress,
    ) -> Result<Option<GetAccountCalldataResponse>, Error> {
        let account_data_url = self.url.join("/accounts/calldata")?;

        let body = json!({
            "address": address
        });

        let response = self
            .client
            .post(account_data_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let response = response.text().await?;
        if response.contains("Address not found") {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_str::<GetAccountCalldataResponse>(&response)?))
        }
    }
}

/// Response from the Cartridge API to fetch the calldata for the constructor of the given
/// controller address.
#[derive(Debug, Clone, Deserialize)]
pub struct GetAccountCalldataResponse {
    /// The address of the controller account.
    pub address: Felt,
    /// The username of the controller account used as salt.
    #[allow(unused)]
    pub username: String,
    /// The calldata for the constructor of the given controller address, this is
    /// UDC calldata, already containing the class hash and the salt + the constructor arguments.
    pub calldata: Vec<Felt>,
}
