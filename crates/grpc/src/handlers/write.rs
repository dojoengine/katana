//! Starknet write service handler implementation.

use katana_pool::TransactionPool;
use katana_primitives::Felt;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_server::starknet::PendingBlockProvider;
use katana_rpc_types::BroadcastedTxWithChainId;
use tonic::{Request, Response, Status};

use crate::error::IntoGrpcResult;
use crate::handlers::StarknetService;
use crate::protos::starknet::starknet_write_server::StarknetWrite;
use crate::protos::starknet::{
    AddDeclareTransactionRequest, AddDeclareTransactionResponse,
    AddDeployAccountTransactionRequest, AddDeployAccountTransactionResponse,
    AddInvokeTransactionRequest, AddInvokeTransactionResponse,
};

#[tonic::async_trait]
impl<Pool, PoolTx, PP, PF> StarknetWrite for StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn add_invoke_transaction(
        &self,
        request: Request<AddInvokeTransactionRequest>,
    ) -> Result<Response<AddInvokeTransactionResponse>, Status> {
        let AddInvokeTransactionRequest { transaction } = request.into_inner();

        let tx = transaction.ok_or(Status::invalid_argument("missing transaction"))?;
        let response = self.inner().add_invoke_tx(tx.try_into()?).await.into_grpc_result()?;

        Ok(Response::new(AddInvokeTransactionResponse {
            transaction_hash: Some(response.transaction_hash.into()),
        }))
    }

    async fn add_declare_transaction(
        &self,
        request: Request<AddDeclareTransactionRequest>,
    ) -> Result<Response<AddDeclareTransactionResponse>, Status> {
        let AddDeclareTransactionRequest { transaction } = request.into_inner();

        let tx = transaction.ok_or(Status::invalid_argument("missing transaction"))?;
        let response = self.inner().add_declare_tx(tx.try_into()?).await.into_grpc_result()?;

        Ok(Response::new(AddDeclareTransactionResponse {
            transaction_hash: Some(response.transaction_hash.into()),
            class_hash: Some(response.class_hash.into()),
        }))
    }

    async fn add_deploy_account_transaction(
        &self,
        request: Request<AddDeployAccountTransactionRequest>,
    ) -> Result<Response<AddDeployAccountTransactionResponse>, Status> {
        let AddDeployAccountTransactionRequest { transaction } = request.into_inner();

        let tx = transaction.ok_or(Status::invalid_argument("missing transaction"))?;
        let response =
            self.inner().add_deploy_account_tx(tx.try_into()?).await.into_grpc_result()?;

        Ok(Response::new(AddDeployAccountTransactionResponse {
            transaction_hash: Some(response.transaction_hash.into()),
            contract_address: Some(Felt::from(response.contract_address).into()),
        }))
    }
}
