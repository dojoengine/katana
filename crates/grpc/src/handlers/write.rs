//! Starknet write service handler implementation.

use tonic::{Request, Response, Status};

use crate::handlers::StarknetHandler;
use crate::protos::starknet::starknet_write_server::StarknetWrite;
use crate::protos::starknet::{
    AddDeclareTransactionRequest, AddDeclareTransactionResponse,
    AddDeployAccountTransactionRequest, AddDeployAccountTransactionResponse,
    AddInvokeTransactionRequest, AddInvokeTransactionResponse,
};

/// Trait for the inner handler that provides Starknet Write API functionality.
#[tonic::async_trait]
pub trait StarknetWriteApiProvider: Clone + Send + Sync + 'static {
    /// Adds an invoke transaction to the pool.
    async fn add_invoke_transaction(
        &self,
        tx: katana_rpc_types::broadcasted::BroadcastedInvokeTx,
    ) -> Result<
        katana_rpc_types::broadcasted::AddInvokeTransactionResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Adds a declare transaction to the pool.
    async fn add_declare_transaction(
        &self,
        tx: katana_rpc_types::broadcasted::BroadcastedDeclareTx,
    ) -> Result<
        katana_rpc_types::broadcasted::AddDeclareTransactionResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Adds a deploy account transaction to the pool.
    async fn add_deploy_account_transaction(
        &self,
        tx: katana_rpc_types::broadcasted::BroadcastedDeployAccountTx,
    ) -> Result<
        katana_rpc_types::broadcasted::AddDeployAccountTransactionResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;
}

#[tonic::async_trait]
impl<T: StarknetWriteApiProvider> StarknetWrite for StarknetHandler<T> {
    async fn add_invoke_transaction(
        &self,
        _request: Request<AddInvokeTransactionRequest>,
    ) -> Result<Response<AddInvokeTransactionResponse>, Status> {
        // Would need to convert proto transaction to RPC transaction
        // For now, return unimplemented
        Err(Status::unimplemented(
            "add_invoke_transaction requires full transaction conversion from proto",
        ))
    }

    async fn add_declare_transaction(
        &self,
        _request: Request<AddDeclareTransactionRequest>,
    ) -> Result<Response<AddDeclareTransactionResponse>, Status> {
        // Would need to convert proto transaction to RPC transaction
        Err(Status::unimplemented(
            "add_declare_transaction requires full transaction conversion from proto",
        ))
    }

    async fn add_deploy_account_transaction(
        &self,
        _request: Request<AddDeployAccountTransactionRequest>,
    ) -> Result<Response<AddDeployAccountTransactionResponse>, Status> {
        // Would need to convert proto transaction to RPC transaction
        Err(Status::unimplemented(
            "add_deploy_account_transaction requires full transaction conversion from proto",
        ))
    }
}
