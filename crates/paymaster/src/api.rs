//! Paymaster JSON-RPC API.
//!
//! NOTE: The `paymaster_rpc` crate already defines an identical `PaymasterApi` trait with the
//! same set of methods. However, it relies on an older version of `jsonrpsee` which is
//! incompatible with the version used in this codebase. As a result, we define our own version
//! of the trait here that uses the same method signatures but is compatible with our `jsonrpsee`
//! version.

use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use paymaster_rpc::{
    BuildTransactionRequest, BuildTransactionResponse, ExecuteRawRequest, ExecuteRawResponse,
    ExecuteRequest, ExecuteResponse, TokenPrice,
};

/// Paymaster API.
#[rpc(client, server, namespace = "paymaster")]
pub trait PaymasterApi {
    /// Health check for the paymaster service.
    #[method(name = "health")]
    async fn health(&self) -> RpcResult<bool>;

    /// Check if the paymaster service is available.
    #[method(name = "isAvailable")]
    async fn is_available(&self) -> RpcResult<bool>;

    /// Build a transaction with paymaster support.
    #[method(name = "buildTransaction")]
    async fn build_transaction(
        &self,
        request: BuildTransactionRequest,
    ) -> RpcResult<BuildTransactionResponse>;

    /// Execute a transaction with paymaster support.
    #[method(name = "executeTransaction")]
    async fn execute_transaction(&self, request: ExecuteRequest) -> RpcResult<ExecuteResponse>;

    /// Execute a raw transaction with paymaster support.
    #[method(name = "executeRawTransaction")]
    async fn execute_raw_transaction(
        &self,
        request: ExecuteRawRequest,
    ) -> RpcResult<ExecuteRawResponse>;

    /// Get the list of supported tokens for gas payment.
    #[method(name = "getSupportedTokens")]
    async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>>;
}
