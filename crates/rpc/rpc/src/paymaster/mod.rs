#![allow(missing_debug_implementations)]

use std::sync::Arc;

use http::Extensions;
use jsonrpsee::core::async_trait;
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};
use paymaster_rpc::context::{Configuration as PaymasterConfiguration, Context as PaymasterContext};
use paymaster_rpc::endpoint::build::build_transaction_endpoint;
use paymaster_rpc::endpoint::execute::execute_endpoint;
use paymaster_rpc::endpoint::health::is_available_endpoint;
use paymaster_rpc::endpoint::token::get_supported_tokens_endpoint;
use paymaster_rpc::endpoint::RequestContext;
use paymaster_rpc::middleware::APIKey;

use katana_rpc_api::paymaster::{BuildTransactionRequest, BuildTransactionResponse, ExecuteRequest, ExecuteResponse, PaymasterApiServer, TokenPrice};

fn map_error(err: katana_rpc_api::paymaster::Error) -> jsonrpsee::core::Error {
    let owned: ErrorObjectOwned = ErrorObject::from(err).into_owned();
    jsonrpsee::core::Error::Call(owned)
}

#[derive(Clone)]
pub struct PaymasterService {
    context: Arc<PaymasterContext>,
    default_api_key: Option<String>,
}

impl PaymasterService {
    pub fn new(configuration: PaymasterConfiguration, default_api_key: Option<String>) -> Self {
        Self { context: Arc::new(PaymasterContext::new(configuration)), default_api_key }
    }

    pub fn context(&self) -> &PaymasterContext {
        &self.context
    }

    fn request_context(&self, api_key: Option<&str>) -> (Extensions, RequestContext<'_>) {
        let mut extensions = Extensions::new();
        if let Some(key) = api_key.or_else(|| self.default_api_key.as_deref()) {
            extensions.insert(APIKey(key.to_string()));
        }

        let request_context = RequestContext::new(&self.context, &extensions);
        (extensions, request_context)
    }
}

pub struct PaymasterRpc {
    service: Arc<PaymasterService>,
}

impl PaymasterRpc {
    pub fn new(service: Arc<PaymasterService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl PaymasterApiServer for PaymasterRpc {
    async fn health(&self) -> RpcResult<bool> {
        Ok(true)
    }

    async fn is_available(&self) -> RpcResult<bool> {
        let (extensions, ctx) = self.service.request_context(None);
        let result = is_available_endpoint(&ctx).await.map_err(map_error);
        drop(extensions);
        result
    }

    async fn build_transaction(&self, params: BuildTransactionRequest) -> RpcResult<BuildTransactionResponse> {
        let (extensions, ctx) = self.service.request_context(None);
        let result = build_transaction_endpoint(&ctx, params).await.map_err(map_error);
        drop(extensions);
        result
    }

    async fn execute_transaction(&self, params: ExecuteRequest) -> RpcResult<ExecuteResponse> {
        let (extensions, ctx) = self.service.request_context(None);
        let result = execute_endpoint(&ctx, params).await.map_err(map_error);
        drop(extensions);
        result
    }

    async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>> {
        let (extensions, ctx) = self.service.request_context(None);
        let result = get_supported_tokens_endpoint(&ctx).await.map_err(map_error);
        drop(extensions);
        result
    }
}
