use jsonrpsee::core::{async_trait, RpcResult};
use katana_executor::{ExecutionResult, ResultAndStates};
use katana_pool::TransactionPool;
use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, TxHash};
use katana_provider::api::block::{BlockNumberProvider, BlockProvider};
use katana_provider::api::transaction::{TransactionTraceProvider, TransactionsProviderExt};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet::StarknetTraceApiServer;
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::trace::{
    to_rpc_fee_estimate, SimulatedTransactions, SimulatedTransactionsResponse,
    TraceBlockTransactionsResponse, TxTrace, TxTraceWithHash,
};
use katana_rpc_types::{BroadcastedTxWithChainId, SimulationFlag};

use super::StarknetApi;
use crate::starknet::pending::PendingBlockProvider;

impl<Pool, PoolTx, Pending> StarknetApi<Pool, Pending>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
{
    fn simulate_txs(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> Result<Vec<SimulatedTransactions>, StarknetApiError> {
        let chain = self.inner.chain_spec.id();

        let executables = transactions
            .into_iter()
            .map(|tx| {
                let is_query = tx.is_query();
                let tx = ExecutableTx::from(BroadcastedTxWithChainId { tx, chain });
                ExecutableTxWithHash::new_query(tx, is_query)
            })
            .collect::<Vec<_>>();

        // If the node is run with transaction validation disabled, then we should not validate
        // even if the `SKIP_VALIDATE` flag is not set.
        let should_validate = !simulation_flags.contains(&SimulationFlag::SkipValidate)
            // && self.inner.backend.executor_factory.execution_flags().account_validation();
        && self.inner.config.simulation_flags.account_validation();

        // If the node is run with fee charge disabled, then we should disable charing fees even
        // if the `SKIP_FEE_CHARGE` flag is not set.
        let should_charge_fee = !simulation_flags.contains(&SimulationFlag::SkipFeeCharge)
            // && self.inner.backend.executor_factory.execution_flags().fee();
        && self.inner.config.simulation_flags.fee();

        let flags = katana_executor::ExecutionFlags::new()
            .with_account_validation(should_validate)
            .with_fee(should_charge_fee)
            .with_nonce_check(false);

        // get the state and block env at the specified block for execution
        let state = self.state(&block_id)?;
        let env = self.block_env_at(&block_id)?;

        // use the blockifier utils function
        // let cfg_env = self.inner.backend.executor_factory.cfg().clone();
        let chain_spec = self.inner.chain_spec.as_ref();
        let cfg_env = self.inner.chain_spec.versioned_constants_overrides().unwrap();
        let results =
            super::blockifier::simulate(chain_spec, state, env, cfg_env, executables, flags);

        let mut simulated = Vec::with_capacity(results.len());
        for (i, ResultAndStates { result, .. }) in results.into_iter().enumerate() {
            match result {
                ExecutionResult::Success { trace, receipt } => {
                    let trace = TypedTransactionExecutionInfo::new(receipt.r#type(), trace);

                    let transaction_trace = TxTrace::from(trace);
                    let fee_estimation =
                        to_rpc_fee_estimate(receipt.resources_used(), receipt.fee());
                    let value = SimulatedTransactions { transaction_trace, fee_estimation };

                    simulated.push(value)
                }

                ExecutionResult::Failed { error } => {
                    return Err(StarknetApiError::transaction_execution_error(
                        i as u64,
                        error.to_string(),
                    ));
                }
            }
        }

        Ok(simulated)
    }

    fn block_traces(
        &self,
        block_id: ConfirmedBlockIdOrTag,
    ) -> Result<Vec<TxTraceWithHash>, StarknetApiError> {
        use StarknetApiError::BlockNotFound;

        let provider = &self.inner.storage;

        let block_id: BlockHashOrNumber = match block_id {
            ConfirmedBlockIdOrTag::L1Accepted => {
                unimplemented!("l1 accepted block id")
            }
            ConfirmedBlockIdOrTag::Latest => provider.latest_number()?.into(),
            ConfirmedBlockIdOrTag::Number(num) => num.into(),
            ConfirmedBlockIdOrTag::Hash(hash) => hash.into(),
        };

        let indices = provider.block_body_indices(block_id)?.ok_or(BlockNotFound)?;
        let tx_hashes = provider.transaction_hashes_in_range(indices.into())?;

        let traces = provider.transaction_executions_by_block(block_id)?.ok_or(BlockNotFound)?;
        let traces = traces.into_iter().map(TxTrace::from);

        let result = tx_hashes
            .into_iter()
            .zip(traces)
            .map(|(h, r)| TxTraceWithHash { transaction_hash: h, trace_root: r })
            .collect::<Vec<_>>();

        Ok(result)
    }

    fn trace(&self, tx_hash: TxHash) -> Result<TxTrace, StarknetApiError> {
        use StarknetApiError::TxnHashNotFound;

        // Check in the pending block first
        if let Some(pending_trace) = self.inner.pending_block_provider.get_pending_trace(tx_hash)? {
            Ok(pending_trace)
        } else {
            // If not found in pending block, fallback to the provider
            let trace =
                self.inner.storage.transaction_execution(tx_hash)?.ok_or(TxnHashNotFound)?;
            Ok(TxTrace::from(trace))
        }
    }
}

#[async_trait]
impl<Pool, PoolTx, Pending> StarknetTraceApiServer for StarknetApi<Pool, Pending>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
{
    async fn trace_transaction(&self, transaction_hash: TxHash) -> RpcResult<TxTrace> {
        self.on_io_blocking_task(move |this| Ok(this.trace(transaction_hash)?)).await?
    }

    async fn simulate_transactions(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<SimulatedTransactionsResponse> {
        self.on_cpu_blocking_task(move |this| async move {
            let transactions = this.simulate_txs(block_id, transactions, simulation_flags)?;
            Ok(SimulatedTransactionsResponse { transactions })
        })
        .await?
    }

    async fn trace_block_transactions(
        &self,
        block_id: ConfirmedBlockIdOrTag,
    ) -> RpcResult<TraceBlockTransactionsResponse> {
        self.on_io_blocking_task(move |this| {
            let traces = this.block_traces(block_id)?;
            Ok(TraceBlockTransactionsResponse { traces })
        })
        .await?
    }
}
