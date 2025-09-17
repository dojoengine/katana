use katana_primitives::class::CasmContractClass;
use katana_primitives::Felt;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::error;
use url::Url;

use crate::types::{Block, BlockId, ContractClass, StateUpdate, StateUpdateWithBlock};

/// HTTP request header for the feeder gateway API key. This allow bypassing the rate limiting.
const X_THROTTLING_BYPASS: &str = "X-Throttling-Bypass";

/// Client for interacting with the Starknet's feeder gateway.
#[derive(Debug, Clone)]
pub struct SequencerGateway {
    /// The feeder gateway base URL.
    base_url: Url,
    /// The HTTP client used to send the requests.
    http_client: Client,
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
        let api_key = None;
        let client = Client::new();
        Self { http_client: client, base_url, api_key }
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
    Sequencer(SequencerError),

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
    Error(SequencerError),
}

#[derive(Debug, thiserror::Error, Deserialize)]
#[error("{message} ({code:?})")]
pub struct SequencerError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ErrorCode {
    #[serde(rename = "StarknetErrorCode.BLOCK_NOT_FOUND")]
    BlockNotFound,
    #[serde(rename = "StarknetErrorCode.ENTRY_POINT_NOT_FOUND_IN_CONTRACT")]
    EntryPointNotFoundInContract,
    #[serde(rename = "StarknetErrorCode.INVALID_PROGRAM")]
    InvalidProgram,
    #[serde(rename = "StarknetErrorCode.TRANSACTION_FAILED")]
    TransactionFailed,
    #[serde(rename = "StarknetErrorCode.TRANSACTION_NOT_FOUND")]
    TransactionNotFound,
    #[serde(rename = "StarknetErrorCode.UNINITIALIZED_CONTRACT")]
    UninitializedContract,
    #[serde(rename = "StarkErrorCode.MALFORMED_REQUEST")]
    MalformedRequest,
    #[serde(rename = "StarknetErrorCode.UNDECLARED_CLASS")]
    UndeclaredClass,
    #[serde(rename = "StarknetErrorCode.INVALID_TRANSACTION_NONCE")]
    InvalidTransactionNonce,
    #[serde(rename = "StarknetErrorCode.VALIDATE_FAILURE")]
    ValidateFailure,
    #[serde(rename = "StarknetErrorCode.CLASS_ALREADY_DECLARED")]
    ClassAlreadyDeclared,
    #[serde(rename = "StarknetErrorCode.COMPILATION_FAILED")]
    CompilationFailed,
    #[serde(rename = "StarknetErrorCode.INVALID_COMPILED_CLASS_HASH")]
    InvalidCompiledClassHash,
    #[serde(rename = "StarknetErrorCode.DUPLICATED_TRANSACTION")]
    DuplicatedTransaction,
    #[serde(rename = "StarknetErrorCode.INVALID_CONTRACT_CLASS")]
    InvalidContractClass,
    #[serde(rename = "StarknetErrorCode.DEPRECATED_ENDPOINT")]
    DeprecatedEndpoint,
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
    async fn send<T: DeserializeOwned>(mut self) -> Result<T, Error> {
        if let Some(block_id) = self.block_id {
            match block_id {
                BlockId::Hash(hash) => {
                    self.url.query_pairs_mut().append_pair("blockHash", &format!("{hash:#x}"));
                }
                BlockId::Number(num) => {
                    self.url.query_pairs_mut().append_pair("blockNumber", &num.to_string());
                }
                BlockId::Latest => {
                    // latest block is implied, if no block id is specified
                }
            }
        }

        let mut headers = HeaderMap::new();

        if let Some(key) = self.gateway_client.api_key.as_ref() {
            let value = HeaderValue::from_str(key)
                .map_err(|_| Error::InvalidHeaderValue { value: key.to_string() })?;
            headers.insert(X_THROTTLING_BYPASS, value);
        }

        let request = self.gateway_client.http_client.get(self.url).headers(headers);
        let response = request.send().await?.error_for_status()?;

        match response.json::<Response<T>>().await? {
            Response::Data(data) => Ok(data),
            Response::Error(error) => Err(Error::Sequencer(error)),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn request_block_id() {
        let base_url = Url::parse("https://example.com/").unwrap();
        let client = SequencerGateway::new(base_url);
        let req = client.feeder_gateway("test");

        // Test block hash
        let hash = Felt::from(123);
        let hash_url = req.clone().block_id(BlockId::Hash(hash)).url;
        assert_eq!(hash_url.query(), Some("blockHash=0x7b"));

        // Test block number
        let num_url = req.clone().block_id(BlockId::Number(42)).url;
        assert_eq!(num_url.query(), Some("blockNumber=42"));

        // Test latest block (should have no query params)
        let latest_url = req.block_id(BlockId::Latest).url;
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
}
