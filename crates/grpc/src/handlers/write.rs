//! Starknet write service handler implementation.

use katana_pool::TransactionPool;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_server::starknet::PendingBlockProvider;
use tonic::{Request, Response, Status};

use crate::handlers::StarknetHandler;
use crate::protos::starknet::starknet_write_server::StarknetWrite;
use crate::protos::starknet::{
    AddDeclareTransactionRequest, AddDeclareTransactionResponse,
    AddDeployAccountTransactionRequest, AddDeployAccountTransactionResponse,
    AddInvokeTransactionRequest, AddInvokeTransactionResponse,
};

#[tonic::async_trait]
impl<Pool, PP, PF> StarknetWrite for StarknetHandler<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn add_invoke_transaction(
        &self,
        _request: Request<AddInvokeTransactionRequest>,
    ) -> Result<Response<AddInvokeTransactionResponse>, Status> {
        // Would need to convert proto transaction to RPC transaction
        Err(Status::unimplemented(
            "add_invoke_transaction requires full transaction conversion from proto",
        ))
    }

    async fn add_declare_transaction(
        &self,
        _request: Request<AddDeclareTransactionRequest>,
    ) -> Result<Response<AddDeclareTransactionResponse>, Status> {
        Err(Status::unimplemented(
            "add_declare_transaction requires full transaction conversion from proto",
        ))
    }

    async fn add_deploy_account_transaction(
        &self,
        _request: Request<AddDeployAccountTransactionRequest>,
    ) -> Result<Response<AddDeployAccountTransactionResponse>, Status> {
        Err(Status::unimplemented(
            "add_deploy_account_transaction requires full transaction conversion from proto",
        ))
    }
}
