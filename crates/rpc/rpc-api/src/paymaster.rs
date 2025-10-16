use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;

pub use paymaster_rpc::client::Client;
pub use paymaster_rpc::endpoint::build::{
    BuildTransactionRequest, BuildTransactionResponse, DeployAndInvokeTransaction, DeployTransaction, FeeEstimate, InvokeParameters, InvokeTransaction,
    TransactionParameters,
};
pub use paymaster_rpc::endpoint::common::{DeploymentParameters, ExecutionParameters, FeeMode, TimeBounds, TipPriority};
pub use paymaster_rpc::endpoint::execute::{ExecutableInvokeParameters, ExecutableTransactionParameters, ExecuteRequest, ExecuteResponse};
pub use paymaster_rpc::endpoint::token::TokenPrice;
pub use paymaster_rpc::Error;

#[cfg_attr(not(feature = "client"), rpc(server))]
#[cfg_attr(feature = "client", rpc(client, server))]
pub trait PaymasterApi {
    #[method(name = "paymaster_health")]
    async fn health(&self) -> RpcResult<bool>;

    #[method(name = "paymaster_isAvailable")]
    async fn is_available(&self) -> RpcResult<bool>;

    #[method(name = "paymaster_buildTransaction")]
    async fn build_transaction(&self, params: BuildTransactionRequest) -> RpcResult<BuildTransactionResponse>;

    #[method(name = "paymaster_executeTransaction")]
    async fn execute_transaction(&self, params: ExecuteRequest) -> RpcResult<ExecuteResponse>;

    #[method(name = "paymaster_getSupportedTokens")]
    async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>>;
}
