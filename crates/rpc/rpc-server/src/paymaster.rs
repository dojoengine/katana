use std::sync::Arc;

use http::{HeaderMap, HeaderValue};
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::error::INTERNAL_ERROR_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use jsonrpsee_024::core::client::ClientT as PaymasterClientT;
use jsonrpsee_024::core::params::ObjectParams as PaymasterObjectParams;
use jsonrpsee_024::core::ClientError as PaymasterClientError;
use jsonrpsee_024::http_client::{
    HttpClient as PaymasterHttpClient, HttpClientBuilder as PaymasterHttpClientBuilder,
};
use paymaster_rpc::{
    BuildTransactionRequest, BuildTransactionResponse, ExecuteRawRequest, ExecuteRawResponse,
    ExecuteRequest, ExecuteResponse, TokenPrice,
};
use serde::Serialize;
use serde_json::Value as JsonValue;
use url::Url;

#[derive(Clone, Debug)]
pub struct PaymasterProxy {
    url: Url,
    api_key: Option<String>,
}

impl PaymasterProxy {
    pub fn new(url: Url, api_key: Option<String>) -> Self {
        Self { url, api_key }
    }

    pub fn module(self) -> Result<RpcModule<()>, jsonrpsee::core::RegisterMethodError> {
        let mut module = RpcModule::new(());
        let proxy = Arc::new(self);

        {
            let proxy = Arc::clone(&proxy);
            module.register_async_method("paymaster_health", move |_, _, _| {
                let proxy = Arc::clone(&proxy);
                async move { proxy.health().await }
            })?;
        }

        {
            let proxy = Arc::clone(&proxy);
            module.register_async_method("paymaster_isAvailable", move |_, _, _| {
                let proxy = Arc::clone(&proxy);
                async move { proxy.is_available().await }
            })?;
        }

        {
            let proxy = Arc::clone(&proxy);
            module.register_async_method("paymaster_buildTransaction", move |params, _, _| {
                let proxy = Arc::clone(&proxy);
                async move {
                    let request: BuildTransactionRequest = params.parse()?;
                    proxy.build_transaction(request).await
                }
            })?;
        }

        {
            let proxy = Arc::clone(&proxy);
            module.register_async_method("paymaster_executeTransaction", move |params, _, _| {
                let proxy = Arc::clone(&proxy);
                async move {
                    let request: ExecuteRequest = params.parse()?;
                    proxy.execute_transaction(request).await
                }
            })?;
        }

        {
            let proxy = Arc::clone(&proxy);
            module.register_async_method("paymaster_executeRawTransaction", move |params, _, _| {
                let proxy = Arc::clone(&proxy);
                async move {
                    let request: ExecuteRawRequest = params.parse()?;
                    proxy.execute_raw_transaction(request).await
                }
            })?;
        }

        {
            let proxy = Arc::clone(&proxy);
            module.register_async_method("paymaster_getSupportedTokens", move |_, _, _| {
                let proxy = Arc::clone(&proxy);
                async move { proxy.get_supported_tokens().await }
            })?;
        }

        Ok(module)
    }

    async fn health(&self) -> RpcResult<bool> {
        let client = self.client()?;
        let params = PaymasterObjectParams::new();
        client
            .request("paymaster_health", params)
            .await
            .map_err(map_client_error)
    }

    async fn is_available(&self) -> RpcResult<bool> {
        let client = self.client()?;
        let params = PaymasterObjectParams::new();
        client
            .request("paymaster_isAvailable", params)
            .await
            .map_err(map_client_error)
    }

    async fn build_transaction(
        &self,
        request: BuildTransactionRequest,
    ) -> RpcResult<BuildTransactionResponse> {
        let client = self.client()?;
        let params = build_request_params(request.transaction, request.parameters)?;
        client
            .request("paymaster_buildTransaction", params)
            .await
            .map_err(map_client_error)
    }

    async fn execute_transaction(&self, request: ExecuteRequest) -> RpcResult<ExecuteResponse> {
        let client = self.client()?;
        let params = build_request_params(request.transaction, request.parameters)?;
        client
            .request("paymaster_executeTransaction", params)
            .await
            .map_err(map_client_error)
    }

    async fn execute_raw_transaction(
        &self,
        request: ExecuteRawRequest,
    ) -> RpcResult<ExecuteRawResponse> {
        let client = self.client()?;
        let params = build_request_params(request.transaction, request.parameters)?;
        client
            .request("paymaster_executeRawTransaction", params)
            .await
            .map_err(map_client_error)
    }

    async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>> {
        let client = self.client()?;
        let params = PaymasterObjectParams::new();
        client
            .request("paymaster_getSupportedTokens", params)
            .await
            .map_err(map_client_error)
    }

    fn client(&self) -> RpcResult<PaymasterHttpClient> {
        let mut headers = HeaderMap::new();
        if let Some(key) = &self.api_key {
            let header_value = HeaderValue::from_str(key).map_err(|err| {
                ErrorObjectOwned::owned(
                    INTERNAL_ERROR_CODE,
                    "Invalid paymaster api key",
                    Some(err.to_string()),
                )
            })?;
            headers.insert("x-paymaster-api-key", header_value);
        }

        PaymasterHttpClientBuilder::default()
            .set_headers(headers)
            .build(self.url.as_str())
            .map_err(map_build_error)
    }
}

fn map_client_error(err: PaymasterClientError) -> ErrorObjectOwned {
    match err {
        PaymasterClientError::Call(err) => convert_error(err),
        other => ErrorObjectOwned::owned(
            INTERNAL_ERROR_CODE,
            "Paymaster proxy error",
            Some(other.to_string()),
        ),
    }
}

fn map_build_error(err: PaymasterClientError) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        INTERNAL_ERROR_CODE,
        "Failed to create paymaster client",
        Some(err.to_string()),
    )
}

fn build_request_params<T, P>(
    transaction: T,
    parameters: P,
) -> Result<PaymasterObjectParams, ErrorObjectOwned>
where
    T: Serialize,
    P: Serialize,
{
    let mut params = PaymasterObjectParams::new();
    params
        .insert("transaction", transaction)
        .map_err(map_param_error)?;
    params
        .insert("parameters", parameters)
        .map_err(map_param_error)?;
    Ok(params)
}

fn map_param_error(err: serde_json::Error) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        INTERNAL_ERROR_CODE,
        "Failed to serialize paymaster params",
        Some(err.to_string()),
    )
}

fn convert_error(err: jsonrpsee_024::types::ErrorObjectOwned) -> ErrorObjectOwned {
    let data = err
        .data()
        .and_then(|raw| serde_json::from_str::<JsonValue>(raw.get()).ok());
    ErrorObjectOwned::owned(err.code(), err.message(), data)
}
