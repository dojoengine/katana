use jsonrpsee::core::{async_trait, RpcResult};
use katana_executor::{ExecutionResult, ExecutorFactory, ResultAndStates};
use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, TxHash};
use katana_provider::api::block::{BlockNumberProvider, BlockProvider};
use katana_provider::api::transaction::{TransactionTraceProvider, TransactionsProviderExt};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet::StarknetTraceApiServer;
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::trace::{
    to_rpc_fee_estimate, SimulatedTransactionsResponse, TxTrace, TxTraceWithHash,
};
use katana_rpc_types::SimulationFlag;

use super::StarknetApi;

impl<EF: ExecutorFactory> StarknetApi<EF> {
    fn simulate_txs(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> Result<Vec<SimulatedTransactionsResponse>, StarknetApiError> {
        let chain_id = self.inner.backend.chain_spec.id();

        let executables = transactions
            .into_iter()
            .map(|tx| {
                let tx = match tx {
                    BroadcastedTx::Invoke(tx) => {
                        let is_query = tx.is_query();
                        ExecutableTxWithHash::new_query(
                            ExecutableTx::Invoke(tx.into_inner(chain_id)),
                            is_query,
                        )
                    }
                    BroadcastedTx::Declare(tx) => {
                        let is_query = tx.is_query();
                        let tx = tx
                            .into_inner(chain_id)
                            .map_err(|_| StarknetApiError::InvalidContractClass)?;

                        ExecutableTxWithHash::new_query(ExecutableTx::Declare(tx), is_query)
                    }
                    BroadcastedTx::DeployAccount(tx) => {
                        let is_query = tx.is_query();
                        ExecutableTxWithHash::new_query(
                            ExecutableTx::DeployAccount(tx.into_inner(chain_id)),
                            is_query,
                        )
                    }
                };
                Result::<ExecutableTxWithHash, StarknetApiError>::Ok(tx)
            })
            .collect::<Result<Vec<_>, _>>()?;

        // If the node is run with transaction validation disabled, then we should not validate
        // even if the `SKIP_VALIDATE` flag is not set.
        let should_validate = !simulation_flags.contains(&SimulationFlag::SkipValidate)
            && self.inner.backend.executor_factory.execution_flags().account_validation();

        // If the node is run with fee charge disabled, then we should disable charing fees even
        // if the `SKIP_FEE_CHARGE` flag is not set.
        let should_charge_fee = !simulation_flags.contains(&SimulationFlag::SkipFeeCharge)
            && self.inner.backend.executor_factory.execution_flags().fee();

        let flags = katana_executor::ExecutionFlags::new()
            .with_account_validation(should_validate)
            .with_fee(should_charge_fee)
            .with_nonce_check(false);

        // get the state and block env at the specified block for execution
        let state = self.state(&block_id)?;
        let env = self.block_env_at(&block_id)?;

        // use the blockifier utils function
        let cfg_env = self.inner.backend.executor_factory.cfg().clone();
        let results = super::blockifier::simulate(state, env, cfg_env, executables, flags);

        let mut simulated = Vec::with_capacity(results.len());
        for (i, ResultAndStates { result, .. }) in results.into_iter().enumerate() {
            match result {
                ExecutionResult::Success { trace, receipt } => {
                    let trace = TypedTransactionExecutionInfo::new(receipt.r#type(), trace);

                    let transaction_trace = TxTrace::from(trace);
                    let fee_estimation =
                        to_rpc_fee_estimate(receipt.resources_used(), receipt.fee());
                    let value = SimulatedTransactionsResponse { transaction_trace, fee_estimation };

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

        let provider = self.inner.backend.blockchain.provider();

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
        if let Some(state) = self.pending_executor() {
            let pending_block = state.read();
            let tx = pending_block.transactions().iter().find(|(t, _)| t.hash == tx_hash);

            if let Some((tx, res)) = tx {
                if let Some(trace) = res.trace() {
                    let trace = TypedTransactionExecutionInfo::new(tx.r#type(), trace.clone());
                    return Ok(TxTrace::from(trace));
                }
            }
        }

        // If not found in pending block, fallback to the provider
        let provider = self.inner.backend.blockchain.provider();
        let trace = provider.transaction_execution(tx_hash)?.ok_or(TxnHashNotFound)?;
        Ok(TxTrace::from(trace))
    }
}

#[async_trait]
impl<EF: ExecutorFactory> StarknetTraceApiServer for StarknetApi<EF> {
    async fn trace_transaction(&self, transaction_hash: TxHash) -> RpcResult<TxTrace> {
        self.on_io_blocking_task(move |this| Ok(this.trace(transaction_hash)?)).await
    }

    async fn simulate_transactions(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulatedTransactionsResponse>> {
        self.on_cpu_blocking_task(move |this| {
            Ok(this.simulate_txs(block_id, transactions, simulation_flags)?)
        })
        .await
    }

    async fn trace_block_transactions(
        &self,
        block_id: ConfirmedBlockIdOrTag,
    ) -> RpcResult<Vec<TxTraceWithHash>> {
        self.on_io_blocking_task(move |this| Ok(this.block_traces(block_id)?)).await
    }
}
