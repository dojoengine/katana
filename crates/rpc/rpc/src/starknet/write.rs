use jsonrpsee::core::{async_trait, RpcResult};
use katana_executor::ExecutorFactory;
use katana_pool::TransactionPool;
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet::StarknetWriteApiServer;
use katana_rpc_types::new_transaction::{
    AddDeclareTransactionResult, AddDeployAccountTransactionResult, AddInvokeTransactionResult,
    BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx,
};

use super::StarknetApi;

impl<EF: ExecutorFactory> StarknetApi<EF> {
    async fn add_invoke_transaction_impl(
        &self,
        tx: BroadcastedInvokeTx,
    ) -> Result<AddInvokeTransactionResult, StarknetApiError> {
        self.on_cpu_blocking_task(move |this| {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let tx = tx.into_inner(this.inner.backend.chain_spec.id());
            let tx = ExecutableTxWithHash::new(ExecutableTx::Invoke(tx));
            let transaction_hash = this.inner.pool.add_transaction(tx)?;

            Ok(AddInvokeTransactionResult { transaction_hash })
        })
        .await
    }

    async fn add_declare_transaction_impl(
        &self,
        tx: BroadcastedDeclareTx,
    ) -> Result<AddDeclareTransactionResult, StarknetApiError> {
        self.on_cpu_blocking_task(move |this| {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let tx = tx.into_inner(this.inner.backend.chain_spec.id());

            let class_hash = tx.class_hash();
            let tx = ExecutableTxWithHash::new(ExecutableTx::Declare(tx));
            let transaction_hash = this.inner.pool.add_transaction(tx)?;

            Ok(AddDeclareTransactionResult { transaction_hash, class_hash })
        })
        .await
    }

    async fn add_deploy_account_transaction_impl(
        &self,
        tx: BroadcastedDeployAccountTx,
    ) -> Result<AddDeployAccountTransactionResult, StarknetApiError> {
        self.on_cpu_blocking_task(move |this| {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let tx = tx.into_inner(this.inner.backend.chain_spec.id());
            let contract_address = tx.contract_address();

            let tx = ExecutableTxWithHash::new(ExecutableTx::DeployAccount(tx));
            let transaction_hash = this.inner.pool.add_transaction(tx)?;

            Ok(AddDeployAccountTransactionResult { transaction_hash, contract_address })
        })
        .await
    }
}

#[async_trait]
impl<EF: ExecutorFactory> StarknetWriteApiServer for StarknetApi<EF> {
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResult> {
        Ok(self.add_invoke_transaction_impl(invoke_transaction).await?)
    }

    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResult> {
        Ok(self.add_declare_transaction_impl(declare_transaction).await?)
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResult> {
        Ok(self.add_deploy_account_transaction_impl(deploy_account_transaction).await?)
    }
}
