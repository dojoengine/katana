use std::marker::PhantomData;
use std::str::FromStr;

use katana_primitives::block::BlockNumber;
use katana_primitives::class::CasmContractClass;
use katana_primitives::Felt;
use katana_rpc_types::{
    BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx, BroadcastedTx,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Request, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tracing::error;
use url::Url;

use crate::types::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, Block, BlockId, BlockSignature, ContractClass, GatewayError,
    PreConfirmedBlock, SequencerPublicKey, StateUpdate, StateUpdateWithBlock,
};

/// HTTP request header for the feeder gateway API key. This allow bypassing the rate limiting.
const X_THROTTLING_BYPASS: &str = "X-Throttling-Bypass";

/// Client for interacting with the Starknet's feeder gateway.
#[derive(Debug, Clone)]
pub struct Client {
    /// The gateway URL.
    gateway: Url,
    /// The feeder gateway URL.
    feeder_gateway: Url,
    /// The API key used to bypass the rate limiting of the feeder gateway.
    api_key: Option<String>,
}

impl Client {
    /// Creates a new gateway client to Starknet mainnet.
    ///
    /// https://docs.starknet.io/tools/important-addresses/#sequencer_base_url
    pub fn mainnet() -> Self {
        Self::new(
            Url::parse("https://alpha-mainnet.starknet.io/gateway").unwrap(),
            Url::parse("https://feeder.alpha-mainnet.starknet.io/feeder_gateway").unwrap(),
        )
    }

    /// Creates a new gateway client to Starknet sepolia testnet.
    ///
    /// https://docs.starknet.io/tools/important-addresses/#sequencer_base_url
    pub fn sepolia() -> Self {
        Self::new(
            Url::parse("https://alpha-sepolia.starknet.io/gateway").unwrap(),
            Url::parse("https://feeder.alpha-sepolia.starknet.io/feeder_gateway").unwrap(),
        )
    }

    /// Creates a new gateway client to Starknet sepolia integration.
    ///
    /// https://docs.starknet.io/tools/important-addresses/#sequencer_base_url
    pub fn sepolia_integration() -> Self {
        Self::new(
            Url::parse("https://integration-sepolia.starknet.io/gateway").unwrap(),
            Url::parse("https://feeder.integration-sepolia.starknet.io/feeder_gateway").unwrap(),
        )
    }

    /// Creates a new client with the given gateway and feeder gateway URLs.
    pub fn new(gateway: Url, feeder_gateway: Url) -> Self {
        Self { gateway, feeder_gateway, api_key: None }
    }

    /// Sets the API key.
    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub async fn get_preconfirmed_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<PreConfirmedBlock, Error> {
        self.feeder_gateway("get_preconfirmed_block")
            .block_id(BlockId::Number(block_number))
            .send()
            .await
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

    pub async fn get_public_key(&self) -> Result<SequencerPublicKey, Error> {
        self.feeder_gateway("get_public_key").send().await
    }

    pub async fn get_signature(&self, block_id: BlockId) -> Result<BlockSignature, Error> {
        self.feeder_gateway("get_signature").block_id(block_id).send().await
    }

    pub async fn add_invoke_transaction(
        &self,
        transaction: BroadcastedInvokeTx,
    ) -> Result<AddInvokeTransactionResponse, Error> {
        self.gateway("add_transaction").json(&BroadcastedTx::Invoke(transaction)).send().await
    }

    pub async fn add_declare_transaction(
        &self,
        transaction: BroadcastedDeclareTx,
    ) -> Result<AddDeclareTransactionResponse, Error> {
        self.gateway("add_transaction").json(&BroadcastedTx::Declare(transaction)).send().await
    }

    pub async fn add_deploy_account_transaction(
        &self,
        transaction: BroadcastedDeployAccountTx,
    ) -> Result<AddDeployAccountTransactionResponse, Error> {
        self.gateway("add_transaction")
            .json(&BroadcastedTx::DeployAccount(transaction))
            .send()
            .await
    }

    /// Creates a [`RequestBuilder`] for a gateway endpoint.
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
    fn gateway(&self, endpoint: &str) -> RequestBuilder<'_, Post> {
        let mut url = self.gateway.clone();
        url.path_segments_mut().expect("invalid base url").push(endpoint);
        RequestBuilder::new(self, url).post()
    }

    /// Creates a [`RequestBuilder`] for a feeder gateway endpoint.
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
        let mut url = self.feeder_gateway.clone();
        url.path_segments_mut().expect("invalid base url").push(endpoint);
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

// Marker types for the RequestBuilder
struct Get;
struct Post;

#[derive(Debug)]
struct RequestBuilder<'a, Method = Get> {
    request: Request,
    client: &'a Client,
    _method: PhantomData<Method>,
}

impl<'a> RequestBuilder<'a> {
    fn new(client: &'a Client, url: Url) -> Self {
        let request = Request::new(Method::GET, url);
        Self { request, client, _method: PhantomData }
    }

    fn post(mut self) -> RequestBuilder<'a, Post> {
        *self.request.method_mut() = Method::POST;
        RequestBuilder { request: self.request, client: self.client, _method: PhantomData }
    }
}

impl RequestBuilder<'_, Post> {
    /// Attach a JSON body to the request.
    fn json<T: serde::Serialize>(mut self, value: &T) -> Self {
        let json = serde_json::to_string_pretty(value).unwrap();
        *self.request.body_mut() = Some(json.into());
        self
    }
}

impl<Method> RequestBuilder<'_, Method> {
    fn block_id(mut self, block_id: BlockId) -> Self {
        match block_id {
            BlockId::Hash(hash) => {
                self.request
                    .url_mut()
                    .query_pairs_mut()
                    .append_pair("blockHash", &format!("{hash:#x}"));
            }
            BlockId::Number(num) => {
                self.request
                    .url_mut()
                    .query_pairs_mut()
                    .append_pair("blockNumber", &num.to_string());
            }
            BlockId::Latest => {
                // latest block is implied, if no block id is specified
            }
            BlockId::Pending => {
                self.request.url_mut().query_pairs_mut().append_pair("blockNumber", "pending");
            }
        }

        self
    }

    /// Adds a query parameter to the request URL.
    fn query_param(mut self, key: &str, value: &str) -> Self {
        self.request.url_mut().query_pairs_mut().append_pair(key, value);
        self
    }
}

impl<Method> RequestBuilder<'_, Method> {
    /// Send the request.
    async fn send<T: DeserializeOwned>(self) -> Result<T, Error> {
        let request = self.build()?;

        let response = reqwest::Client::new().execute(request).await?;
        match response.json().await? {
            Response::Data(data) => Ok(data),
            Response::Error(error) => Err(Error::Sequencer(error)),
        }
    }

    // build the request
    fn build(self) -> Result<Request, Error> {
        let mut request = self.request;

        if let Some(value) = self.client.api_key.as_ref() {
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

    use serde_json::json;

    use super::*;

    fn test_client() -> Client {
        let gateway = Url::parse("https://example.com/gateway").unwrap();
        let feeder = Url::parse("https://example.com/feeder_gateway").unwrap();

        Client::new(gateway, feeder)
    }

    #[test]
    fn request_block_id() {
        let client = test_client();

        // Test block hash
        let hash = Felt::from(123);
        let builder = client.feeder_gateway("foo").block_id(BlockId::Hash(hash));
        let hash_url = builder.request.url();
        assert_eq!(hash_url.query(), Some("blockHash=0x7b"));

        // Test block number
        let builder = client.feeder_gateway("foo").block_id(BlockId::Number(42));
        let num_url = builder.request.url();
        assert_eq!(num_url.query(), Some("blockNumber=42"));

        // Test latest block (should have no query params)
        let builder = client.feeder_gateway("foo").block_id(BlockId::Latest);
        let latest_url = builder.request.url();
        assert_eq!(latest_url.query(), None);
    }

    #[test]
    fn multiple_query_params() {
        let client = test_client();
        let request = client
            .feeder_gateway("test")
            .query_param("param1", "value1")
            .query_param("param2", "value2")
            .query_param("param3", "value3")
            .request;

        let url = request.url();
        let query = url.query().unwrap();

        assert!(query.contains("param1=value1"));
        assert!(query.contains("param2=value2"));
        assert!(query.contains("param3=value3"));
    }

    #[test]
    fn api_key_header() {
        let gateway = Url::parse("https://example.com/gateway").unwrap();
        let feeder = Url::parse("https://example.com/feeder_gateway").unwrap();

        // Test with API key set
        let api_key = "test-api-key-12345";
        let client_with_key =
            Client::new(gateway.clone(), feeder.clone()).with_api_key(api_key.to_string());
        let request = client_with_key.feeder_gateway("test").build().unwrap();

        // Check that the X-Throttling-Bypass header is set with the correct API key
        let headers = request.headers();
        assert_eq!(headers.get(X_THROTTLING_BYPASS).unwrap().to_str().unwrap(), api_key);

        // Test without API key
        let client_without_key = Client::new(gateway, feeder);
        let request = client_without_key.feeder_gateway("test").build().unwrap();

        // Check that the X-Throttling-Bypass header is not present
        let headers = request.headers();
        assert!(headers.get(X_THROTTLING_BYPASS).is_none());
    }

    #[test]
    fn request_body() {
        let client = test_client();
        let body = json!({ "key": "value" });

        let builder = client.gateway("test").json(&body);
        let request_body = builder.request.body().unwrap();

        let expected_body = serde_json::to_string_pretty(&body).unwrap();
        assert_eq!(request_body.as_bytes().unwrap(), expected_body.as_bytes());
    }
}
