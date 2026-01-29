//! Paymaster proxy implementation.

use http::{HeaderMap, HeaderName, HeaderValue};
use jsonrpsee::core::{async_trait, ClientError, RpcResult};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::types::error::INTERNAL_ERROR_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use katana_rpc_api::paymaster::{PaymasterApiClient, PaymasterApiServer};
use paymaster_rpc::{
    BuildTransactionRequest, BuildTransactionResponse, ExecuteRawRequest, ExecuteRawResponse,
    ExecuteRequest, ExecuteResponse, TokenPrice,
};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum PaymasterProxyError {
    #[error("invalid API key")]
    InvalidApiKey(#[from] http::header::InvalidHeaderValue),

    #[error("client error")]
    Client(#[from] jsonrpsee::core::ClientError),
}

/// Paymaster proxy that forwards requests to an external paymaster service.
#[derive(Clone, Debug)]
pub struct PaymasterProxy {
    upstream_client: HttpClient,
}

impl PaymasterProxy {
    pub fn new(url: Url, api_key: Option<String>) -> Result<Self, PaymasterProxyError> {
        let headers = if let Some(api_key) = &api_key {
            let name = HeaderName::from_static("x-paymaster-api-key");
            let value = HeaderValue::from_str(api_key)?;
            HeaderMap::from_iter([(name, value)])
        } else {
            HeaderMap::default()
        };

        let client = HttpClientBuilder::default().set_headers(headers).build(url.as_str())?;

        Ok(Self { upstream_client: client })
    }
}

#[async_trait]
impl PaymasterApiServer for PaymasterProxy {
    async fn health(&self) -> RpcResult<bool> {
        PaymasterApiClient::health(&self.upstream_client).await.map_err(map_error)
    }

    async fn is_available(&self) -> RpcResult<bool> {
        PaymasterApiClient::is_available(&self.upstream_client).await.map_err(map_error)
    }

    async fn build_transaction(
        &self,
        req: BuildTransactionRequest,
    ) -> RpcResult<BuildTransactionResponse> {
        PaymasterApiClient::build_transaction(&self.upstream_client, req).await.map_err(map_error)
    }

    async fn execute_transaction(&self, req: ExecuteRequest) -> RpcResult<ExecuteResponse> {
        PaymasterApiClient::execute_transaction(&self.upstream_client, req).await.map_err(map_error)
    }

    async fn execute_raw_transaction(
        &self,
        req: ExecuteRawRequest,
    ) -> RpcResult<ExecuteRawResponse> {
        PaymasterApiClient::execute_raw_transaction(&self.upstream_client, req)
            .await
            .map_err(map_error)
    }

    async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>> {
        PaymasterApiClient::get_supported_tokens(&self.upstream_client).await.map_err(map_error)
    }
}

fn map_error(err: ClientError) -> ErrorObjectOwned {
    match err {
        ClientError::Call(err) => err,
        other => ErrorObjectOwned::owned(
            INTERNAL_ERROR_CODE,
            "Paymaster proxy error",
            Some(other.to_string()),
        ),
    }
}
