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
pub struct Client {
    url: Url,
    client: reqwest::Client,
}

impl Client {
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
    pub address: ContractAddress,
    /// The username of the controller account used as salt.
    pub username: String,
    /// The calldata for the constructor of the given controller address, this is
    /// UDC calldata, already containing the class hash and the salt + the constructor arguments.
    #[serde(rename = "calldata")]
    pub constructor_calldata: Vec<Felt>,
}

#[cfg(test)]
mod tests {
    use katana_primitives::{address, ContractAddress};

    use super::*;

    #[test]
    fn deserialize_get_account_calldata_response() {
        let response = json!({
            "address": "0x48e13ef7ab79637afd38a4b022862a7e6f3fd934f194c435d7e7b17bac06715",
            "username": "glihm",
            "calldata": [
                "0x24a9edbfa7082accfceabf6a92d7160086f346d622f28741bf1c651c412c9ab",
                "0x676c69686d",
                "0x0",
                "0x1e",
                "0x0",
                "0x4",
                "0x16",
                "0x68",
                "0x74",
                "0x74",
                "0x70",
                "0x73",
                "0x3a",
                "0x2f",
                "0x2f",
                "0x78",
                "0x2e",
                "0x63",
                "0x61",
                "0x72",
                "0x74",
                "0x72",
                "0x69",
                "0x64",
                "0x67",
                "0x65",
                "0x2e",
                "0x67",
                "0x67",
                "0x9d0aec9905466c9adf79584fa75fed3",
                "0x20a97ec3f8efbc2aca0cf7cabb420b4a",
                "0x30910fae3f3451a26071c3afc453425e",
                "0xa4e54fa48a6c3f34444687c2552b157f",
                "0x1"
            ]
        });

        let response: GetAccountCalldataResponse = serde_json::from_value(response).unwrap();

        assert_eq!(
            response.address,
            address!("0x48e13ef7ab79637afd38a4b022862a7e6f3fd934f194c435d7e7b17bac06715")
        );

        assert_eq!(response.username, "glihm");
        assert_eq!(response.constructor_calldata.len(), 34);
    }
}
