use jsonrpsee::core::{async_trait, RpcResult};
use katana_pool::TransactionPool;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet::StarknetWriteApiServer;
use katana_rpc_types::broadcasted::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, BroadcastedDeclareTx, BroadcastedDeployAccountTx,
    BroadcastedInvokeTx,
};
use katana_rpc_types::{BroadcastedTx, BroadcastedTxWithChainId};

use crate::starknet::pending::PendingBlockProvider;

impl<Pool, PoolTx, Pending> StarknetApi<Pool, Pending>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
{
    async fn add_invoke_transaction_impl(
        &self,
        tx: BroadcastedInvokeTx,
    ) -> Result<AddInvokeTransactionResponse, StarknetApiError> {
        self.on_cpu_blocking_task(|this| async move {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let chain_id = this.inner.chain_spec.id();
            let tx = BroadcastedTxWithChainId { tx: BroadcastedTx::Invoke(tx), chain: chain_id };

            let transaction_hash = this.inner.pool.add_transaction(tx.into()).await?;
            Ok(AddInvokeTransactionResponse { transaction_hash })
        })
        .await?
    }

    async fn add_declare_transaction_impl(
        &self,
        tx: BroadcastedDeclareTx,
    ) -> Result<AddDeclareTransactionResponse, StarknetApiError> {
        self.on_cpu_blocking_task(|this| async move {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let chain_id = this.inner.chain_spec.id();
            let class_hash =
                tx.contract_class.hash().map_err(|_| StarknetApiError::InvalidContractClass)?;

            let tx = BroadcastedTxWithChainId { tx: BroadcastedTx::Declare(tx), chain: chain_id };

            let transaction_hash = this.inner.pool.add_transaction(tx.into()).await?;
            Ok(AddDeclareTransactionResponse { transaction_hash, class_hash })
        })
        .await?
    }

    async fn add_deploy_account_transaction_impl(
        &self,
        tx: BroadcastedDeployAccountTx,
    ) -> Result<AddDeployAccountTransactionResponse, StarknetApiError> {
        self.on_cpu_blocking_task(|this| async move {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let chain_id = this.inner.chain_spec.id();
            let contract_address = tx.contract_address();
            let tx =
                BroadcastedTxWithChainId { tx: BroadcastedTx::DeployAccount(tx), chain: chain_id };

            let transaction_hash = this.inner.pool.add_transaction(tx.into()).await?;
            Ok(AddDeployAccountTransactionResponse { transaction_hash, contract_address })
        })
        .await?
    }
}

#[async_trait]
impl<Pool, PoolTx, Pending> StarknetWriteApiServer for StarknetApi<Pool, Pending>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
{
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        Ok(self.add_invoke_transaction_impl(invoke_transaction).await?)
    }

    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse> {
        Ok(self.add_declare_transaction_impl(declare_transaction).await?)
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse> {
        Ok(self.add_deploy_account_transaction_impl(deploy_account_transaction).await?)
    }
}
