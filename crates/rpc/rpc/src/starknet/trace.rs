use jsonrpsee::core::{async_trait, RpcResult};
use katana_executor::{ExecutionResult, ExecutorFactory, ResultAndStates};
use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag};
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, TxHash};
use katana_provider::traits::block::{BlockNumberProvider, BlockProvider};
use katana_provider::traits::transaction::{TransactionTraceProvider, TransactionsProviderExt};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet::StarknetTraceApiServer;
use katana_rpc_types::trace::{to_rpc_fee_estimate, to_rpc_trace};
use katana_rpc_types::transaction::BroadcastedTx;
use katana_rpc_types::SimulationFlag;
use starknet::core::types::{
    BlockTag, SimulatedTransaction, TransactionTrace, TransactionTraceWithHash,
};

use super::StarknetApi;

impl<EF: ExecutorFactory> StarknetApi<EF> {
    fn simulate_txs(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> Result<Vec<SimulatedTransaction>, StarknetApiError> {
        let chain_id = self.inner.backend.chain_spec.id();

        let executables = transactions
            .into_iter()
            .map(|tx| {
                let tx = match tx {
                    BroadcastedTx::Invoke(tx) => {
                        let is_query = tx.is_query();
                        ExecutableTxWithHash::new_query(
                            ExecutableTx::Invoke(tx.into_tx_with_chain_id(chain_id)),
                            is_query,
                        )
                    }
                    BroadcastedTx::Declare(tx) => {
                        let is_query = tx.is_query();
                        ExecutableTxWithHash::new_query(
                            ExecutableTx::Declare(
                                tx.try_into_tx_with_chain_id(chain_id)
                                    .map_err(|_| StarknetApiError::InvalidContractClass)?,
                            ),
                            is_query,
                        )
                    }
                    BroadcastedTx::DeployAccount(tx) => {
                        let is_query = tx.is_query();
                        ExecutableTxWithHash::new_query(
                            ExecutableTx::DeployAccount(tx.into_tx_with_chain_id(chain_id)),
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

        // create the executor
        let executor = self.inner.backend.executor_factory.with_state_and_block_env(state, env);
        let results = executor.simulate(executables, flags);

        let mut simulated = Vec::with_capacity(results.len());
        for (i, ResultAndStates { result, .. }) in results.into_iter().enumerate() {
            match result {
                ExecutionResult::Success { trace, receipt } => {
                    let trace = TypedTransactionExecutionInfo::new(receipt.r#type(), trace);

                    let transaction_trace = to_rpc_trace(trace);
                    let fee_estimation = to_rpc_fee_estimate(receipt.fee().clone());
                    let value = SimulatedTransaction { transaction_trace, fee_estimation };

                    simulated.push(value)
                }

                ExecutionResult::Failed { error } => {
                    let error = StarknetApiError::TransactionExecutionError {
                        transaction_index: i as u64,
                        execution_error: error.to_string(),
                    };
                    return Err(error);
                }
            }
        }

        Ok(simulated)
    }

    fn block_traces(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<Vec<TransactionTraceWithHash>, StarknetApiError> {
        use StarknetApiError::BlockNotFound;

        let provider = self.inner.backend.blockchain.provider();

        let block_id: BlockHashOrNumber = match block_id {
            BlockIdOrTag::Tag(BlockTag::Pending) => match self.pending_executor() {
                Some(state) => {
                    let pending_block = state.read();

                    // extract the txs from the pending block
                    let traces = pending_block.transactions().iter().filter_map(|(t, r)| {
                        if let Some(trace) = r.trace().cloned() {
                            let transaction_hash = t.hash;
                            let trace = TypedTransactionExecutionInfo::new(t.r#type(), trace);
                            let trace_root = to_rpc_trace(trace);

                            Some(TransactionTraceWithHash { transaction_hash, trace_root })
                        } else {
                            None
                        }
                    });

                    return Ok(traces.collect::<Vec<TransactionTraceWithHash>>());
                }

                // if there is no pending block, return the latest block
                None => provider.latest_number()?.into(),
            },
            BlockIdOrTag::Tag(BlockTag::Latest) => provider.latest_number()?.into(),
            BlockIdOrTag::Number(num) => num.into(),
            BlockIdOrTag::Hash(hash) => hash.into(),
        };

        let indices = provider.block_body_indices(block_id)?.ok_or(BlockNotFound)?;
        let tx_hashes = provider.transaction_hashes_in_range(indices.into())?;

        let traces = provider.transaction_executions_by_block(block_id)?.ok_or(BlockNotFound)?;
        let traces = traces.into_iter().map(to_rpc_trace);

        let result = tx_hashes
            .into_iter()
            .zip(traces)
            .map(|(h, r)| TransactionTraceWithHash { transaction_hash: h, trace_root: r })
            .collect::<Vec<_>>();

        Ok(result)
    }

    fn trace(&self, tx_hash: TxHash) -> Result<TransactionTrace, StarknetApiError> {
        use StarknetApiError::TxnHashNotFound;

        // Check in the pending block first
        if let Some(state) = self.pending_executor() {
            let pending_block = state.read();
            let tx = pending_block.transactions().iter().find(|(t, _)| t.hash == tx_hash);

            if let Some((tx, res)) = tx {
                if let Some(trace) = res.trace() {
                    let trace = TypedTransactionExecutionInfo::new(tx.r#type(), trace.clone());
                    return Ok(to_rpc_trace(trace));
                }
            }
        }

        // If not found in pending block, fallback to the provider
        let provider = self.inner.backend.blockchain.provider();
        let trace = provider.transaction_execution(tx_hash)?.ok_or(TxnHashNotFound)?;
        Ok(to_rpc_trace(trace))
    }
}

#[async_trait]
impl<EF: ExecutorFactory> StarknetTraceApiServer for StarknetApi<EF> {
    async fn trace_transaction(&self, transaction_hash: TxHash) -> RpcResult<TransactionTrace> {
        self.on_io_blocking_task(move |this| Ok(this.trace(transaction_hash)?)).await
    }

    async fn simulate_transactions(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulatedTransaction>> {
        self.on_cpu_blocking_task(move |this| {
            Ok(this.simulate_txs(block_id, transactions, simulation_flags)?)
        })
        .await
    }

    async fn trace_block_transactions(
        &self,
        block_id: BlockIdOrTag,
    ) -> RpcResult<Vec<TransactionTraceWithHash>> {
        self.on_io_blocking_task(move |this| Ok(this.block_traces(block_id)?)).await
    }
}
