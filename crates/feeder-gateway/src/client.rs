use std::str::FromStr;

use katana_primitives::class::CasmContractClass;
use katana_primitives::Felt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, Request, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tracing::error;
use url::Url;

use crate::types::{
    Block, BlockId, ContractClass, GatewayError, StateUpdate, StateUpdateWithBlock,
};

/// HTTP request header for the feeder gateway API key. This allow bypassing the rate limiting.
const X_THROTTLING_BYPASS: &str = "X-Throttling-Bypass";

/// Client for interacting with the Starknet's feeder gateway.
#[derive(Debug, Clone)]
pub struct SequencerGateway {
    /// The feeder gateway base URL.
    base_url: Url,
    /// The API key used to bypass the rate limiting of the feeder gateway.
    api_key: Option<String>,
}

impl SequencerGateway {
    /// Creates a new gateway client to Starknet mainnet.
    ///
    /// https://docs.starknet.io/tools/important-addresses/#sequencer_base_url
    pub fn sn_mainnet() -> Self {
        Self::new(Url::parse("https://feeder.alpha-mainnet.starknet.io/").unwrap())
    }

    /// Creates a new gateway client to Starknet sepolia.
    ///
    /// https://docs.starknet.io/tools/important-addresses/#sequencer_base_url
    pub fn sn_sepolia() -> Self {
        Self::new(Url::parse("https://feeder.integration-sepolia.starknet.io/").unwrap())
    }

    /// Creates a new gateway client at the given base URL.
    pub fn new(base_url: Url) -> Self {
        Self { base_url, api_key: None }
    }

    /// Sets the API key.
    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub async fn get_block(&self, block_id: BlockId) -> Result<Block, Error> {
        self.feeder_gateway("get_block").block_id(block_id).send().await
    }

    pub async fn get_state_update(&self, block_id: BlockId) -> Result<StateUpdate, Error> {
        self.feeder_gateway("get_state_update").block_id(block_id).send().await
    }

    pub async fn get_state_update_with_block(
        &self,
        block_id: BlockId,
    ) -> Result<StateUpdateWithBlock, Error> {
        self.feeder_gateway("get_state_update")
            .query_param("includeBlock", "true")
            .block_id(block_id)
            .send()
            .await
    }

    pub async fn get_class(&self, hash: Felt, block_id: BlockId) -> Result<ContractClass, Error> {
        self.feeder_gateway("get_class_by_hash")
            .query_param("classHash", &format!("{hash:#x}"))
            .block_id(block_id)
            .send()
            .await
    }

    pub async fn get_compiled_class(
        &self,
        hash: Felt,
        block_id: BlockId,
    ) -> Result<CasmContractClass, Error> {
        self.feeder_gateway("get_compiled_class_by_class_hash")
            .query_param("classHash", &format!("{hash:#x}"))
            .block_id(block_id)
            .send()
            .await
    }

    /// Creates a [`RequestBuilder`] for a feeder gateway endpoint.
    ///
    /// This method constructs a URL by appending "feeder_gateway" and the specified endpoint
    /// to the base URL, then returns a [`RequestBuilder`] that can be used to build and send the
    /// request.
    ///
    /// ## Arguments
    ///
    /// * `endpoint` - The specific feeder gateway endpoint to call (e.g., "get_block",
    ///   "get_state_update")
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let gateway = SequencerGateway::sn_mainnet();
    /// let request = gateway.feeder_gateway("get_block")
    ///     .block_id(BlockId::Latest)
    ///     .send()
    ///     .await?;
    /// ```
    fn feeder_gateway(&self, endpoint: &str) -> RequestBuilder<'_> {
        let mut url = self.base_url.clone();
        url.path_segments_mut().expect("invalid base url").extend(["feeder_gateway", endpoint]);
        RequestBuilder::new(self, url)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Network(reqwest::Error),

    #[error(transparent)]
    Sequencer(GatewayError),

    #[error("failed to parse header value '{value}'")]
    InvalidHeaderValue { value: String },

    #[error("request rate limited")]
    RateLimited,
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        if let Some(status) = err.status() {
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Self::RateLimited;
            }
        }

        Self::Network(err)
    }
}

impl Error {
    /// Returns `true` if the error is due to rate limiting.
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Response<T> {
    Data(T),
    Error(GatewayError),
}

#[derive(Debug, Clone)]
struct RequestBuilder<'a> {
    gateway_client: &'a SequencerGateway,
    block_id: Option<BlockId>,
    url: Url,
}

impl<'a> RequestBuilder<'a> {
    fn new(gateway_client: &'a SequencerGateway, url: Url) -> Self {
        Self { gateway_client, block_id: None, url }
    }

    fn block_id(mut self, block_id: BlockId) -> Self {
        self.block_id = Some(block_id);
        self
    }

    /// Adds a query parameter to the request URL.
    fn query_param(mut self, key: &str, value: &str) -> Self {
        self.url.query_pairs_mut().append_pair(key, value);
        self
    }
}

impl RequestBuilder<'_> {
    /// Send the request.
    async fn send<T: DeserializeOwned>(self) -> Result<T, Error> {
        let request = self.build()?;
        let response = Client::new().execute(request).await?;
        match response.json().await? {
            Response::Data(data) => Ok(data),
            Response::Error(error) => Err(Error::Sequencer(error)),
        }
    }

    /// Build the request.
    fn build(self) -> Result<Request, Error> {
        let mut url = self.url;

        if let Some(id) = self.block_id {
            match id {
                BlockId::Hash(hash) => {
                    url.query_pairs_mut().append_pair("blockHash", &format!("{hash:#x}"));
                }
                BlockId::Number(num) => {
                    url.query_pairs_mut().append_pair("blockNumber", &num.to_string());
                }
                BlockId::Latest => {
                    // latest block is implied, if no block id is specified
                }
            }
        }

        let mut request = Request::new(Method::GET, url);

        if let Some(value) = self.gateway_client.api_key.as_ref() {
            let key = HeaderName::from_str(X_THROTTLING_BYPASS).expect("valid header name");
            let value = HeaderValue::from_str(value)
                .map_err(|_| Error::InvalidHeaderValue { value: value.to_string() })?;

            *request.headers_mut() = HeaderMap::from_iter([(key, value)]);
        }

        Ok(request)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn request_block_id() {
        let base_url = Url::parse("https://example.com/").unwrap();
        let client = SequencerGateway::new(base_url);
        let builder = client.feeder_gateway("test");

        // Test block hash
        let hash = Felt::from(123);
        let req = builder.clone().block_id(BlockId::Hash(hash)).build().unwrap();
        let hash_url = req.url();
        assert_eq!(hash_url.query(), Some("blockHash=0x7b"));

        // Test block number
        let req = builder.clone().block_id(BlockId::Number(42)).build().unwrap();
        let num_url = req.url();
        assert_eq!(num_url.query(), Some("blockNumber=42"));

        // Test latest block (should have no query params)
        let req = builder.clone().block_id(BlockId::Latest).build().unwrap();
        let latest_url = req.url();
        assert_eq!(latest_url.query(), None);
    }

    #[test]
    fn multiple_query_params() {
        let base_url = Url::parse("https://example.com/").unwrap();
        let client = SequencerGateway::new(base_url);
        let req = client.feeder_gateway("test");

        let url = req
            .query_param("param1", "value1")
            .query_param("param2", "value2")
            .query_param("param3", "value3")
            .url;

        let query = url.query().unwrap();
        assert!(query.contains("param1=value1"));
        assert!(query.contains("param2=value2"));
        assert!(query.contains("param3=value3"));
    }

    #[test]
    fn api_key_header() {
        let url = Url::parse("https://example.com/").unwrap();

        // Test with API key set
        let api_key = "test-api-key-12345";
        let client_with_key = SequencerGateway::new(url.clone()).with_api_key(api_key.to_string());
        let req = client_with_key.feeder_gateway("test").build().unwrap();

        // Check that the X-Throttling-Bypass header is set with the correct API key
        let headers = req.headers();
        assert_eq!(headers.get(X_THROTTLING_BYPASS).unwrap().to_str().unwrap(), api_key);

        // Test without API key
        let client_without_key = SequencerGateway::new(url);
        let req = client_without_key.feeder_gateway("test").build().unwrap();

        // Check that the X-Throttling-Bypass header is not present
        let headers = req.headers();
        assert!(headers.get(X_THROTTLING_BYPASS).is_none());
    }
}
