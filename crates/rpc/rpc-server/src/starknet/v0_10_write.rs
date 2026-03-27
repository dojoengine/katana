//! v0.10 Write API implementation.
//!
//! Write API types are identical between v0.9 and v0.10 — thin delegation.

use jsonrpsee::core::{async_trait, RpcResult};
use katana_pool::TransactionPool;
use katana_provider::ProviderFactory;
use katana_rpc_api::starknet::v0_10::StarknetWriteApiServer;
use katana_rpc_types::broadcasted::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, BroadcastedDeclareTx, BroadcastedDeployAccountTx,
    BroadcastedInvokeTx,
};
use katana_rpc_types::BroadcastedTxWithChainId;

use super::StarknetApi;
use crate::starknet::pending::PendingBlockProvider;

#[async_trait]
impl<Pool, PoolTx, Pending, PF> StarknetWriteApiServer for StarknetApi<Pool, Pending, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
    PF: ProviderFactory,
{
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        Ok(self.add_invoke_tx(invoke_transaction).await?)
    }

    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse> {
        Ok(self.add_declare_tx(declare_transaction).await?)
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse> {
        Ok(self.add_deploy_account_tx(deploy_account_transaction).await?)
    }
}
