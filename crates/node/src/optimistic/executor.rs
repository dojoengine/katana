use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::stream::StreamExt;
use katana_core::backend::storage::Blockchain;
use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{ExecutionResult, ExecutorFactory};
use katana_pool::ordering::FiFo;
use katana_pool::{PendingTransactions, PoolOrd, PoolTransaction, TransactionPool};
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::providers::db::cached::CachedDbProvider;
use katana_rpc_types::{BroadcastedTx, BroadcastedTxWithChainId};
use katana_tasks::{JoinHandle, TaskSpawner};
use tracing::{debug, error, info, trace, warn};

use crate::optimistic::pool::TxPool;

const LOG_TARGET: &str = "optimistic_executor";

#[derive(Debug)]
pub struct OptimisticExecutor {
    pool: TxPool,
    optimistic_state: CachedDbProvider<katana_db::Db>,
    executor_factory: Arc<BlockifierFactory>,
    task_spawner: TaskSpawner,
}

impl OptimisticExecutor {
    /// Creates a new `OptimisticExecutor` instance.
    ///
    /// # Arguments
    ///
    /// * `pool` - The transaction pool to monitor for new transactions
    /// * `backend` - The backend containing the executor factory and blockchain state
    /// * `task_spawner` - The task spawner used to run the executor actor
    pub fn new(
        pool: TxPool,
        optimistic_state: CachedDbProvider<katana_db::Db>,
        executor_factory: Arc<BlockifierFactory>,
        task_spawner: TaskSpawner,
    ) -> Self {
        Self { pool, optimistic_state, executor_factory, task_spawner }
    }

    /// Spawns the optimistic executor actor task.
    ///
    /// This method creates a subscription to the pool's pending transactions and spawns
    /// an async task that continuously processes incoming transactions.
    ///
    /// # Returns
    ///
    /// A `JoinHandle` to the spawned executor task.
    pub fn spawn(self) -> JoinHandle<()> {
        let actor =
            OptimisticExecutorActor::new(self.pool, self.optimistic_state, self.executor_factory);
        self.task_spawner.build_task().name("Optimistic Executor").spawn(actor)
    }
}

#[derive(Debug)]
struct OptimisticExecutorActor {
    pool: TxPool,
    optimistic_state: CachedDbProvider<katana_db::Db>,
    /// Stream of pending transactions from the pool
    pending_txs: PendingTransactions<BroadcastedTx, FiFo<BroadcastedTx>>,
    storage: Blockchain,
    executor_factory: Arc<BlockifierFactory>,
}

impl OptimisticExecutorActor {
    /// Creates a new executor actor with the given pending transactions stream.
    fn new(
        pool: TxPool,
        optimistic_state: CachedDbProvider<katana_db::Db>,
        executor_factory: Arc<BlockifierFactory>,
    ) -> Self {
        let pending_txs = pool.pending_transactions();
        Self { pool, optimistic_state, pending_txs, storage, executor_factory }
    }

    /// Execute a single transaction optimistically against the latest state.
    fn execute_transaction(&self, tx: BroadcastedTxWithChainId) -> Result<ExecutionResult, String> {
        let latest_state = self.optimistic_state.latest().unwrap();
        let mut executor = self.executor_factory.with_state(latest_state);

        // Execute the transaction
        let result = executor.execute_transactions(vec![tx.clone()]);

        match result {
            Ok((executed_count, limit_error)) => {
                if executed_count == 0 {
                    return Err("Transaction was not executed".to_string());
                }

                // Get the execution result from the executor
                let transactions = executor.transactions();
                if let Some((_, exec_result)) = transactions.last() {
                    if let Some(err) = limit_error {
                        warn!(
                            target: LOG_TARGET,
                            tx_hash = format!("{:#x}", tx.hash),
                            error = %err,
                            "Transaction execution hit limits"
                        );
                    }
                    Ok(exec_result.clone())
                } else {
                    Err("No execution result found".to_string())
                }

                let output = executor.take_execution_output().unwrap();
                self.optimistic_state.merge_state_updates(&output.states);

                // remove from pool
                self.pool.remove_transactions();
            }

            Err(e) => Err(format!("Execution failed: {e}")),
        }
    }
}

impl Future for OptimisticExecutorActor {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Drain all available transactions from the stream until it's exhausted (Poll::Pending)
        // or the stream ends (Poll::Ready(None)).
        //
        // This ensures we process all pending transactions in a batch before yielding control
        // back to the executor, which is more efficient than processing one transaction at a
        // time.
        loop {
            match this.pending_txs.poll_next_unpin(cx) {
                Poll::Ready(Some(pending_tx)) => {
                    let tx = pending_tx.tx.as_ref().clone();

                    let tx_hash = tx.hash;
                    let tx_sender = tx.sender();
                    let tx_nonce = tx.nonce();

                    trace!(
                        target: LOG_TARGET,
                        tx_hash = format!("{:#x}", tx_hash),
                        sender = %tx_sender,
                        nonce = %tx_nonce,
                        "Received transaction from pool"
                    );

                    debug!(
                        target: LOG_TARGET,
                        tx_hash = format!("{:#x}", tx_hash),
                        "Executing transaction optimistically"
                    );

                    // Execute the transaction optimistically
                    match this.execute_transaction(tx) {
                        Ok(ExecutionResult::Success { receipt, .. }) => {
                            if let Some(reason) = receipt.revert_reason() {
                                warn!(
                                    target: LOG_TARGET,
                                    tx_hash = format!("{:#x}", tx_hash),
                                    reason = %reason,
                                    "Transaction reverted"
                                );
                            } else {
                                debug!(
                                    target: LOG_TARGET,
                                    tx_hash = format!("{:#x}", tx_hash),
                                    l1_gas = receipt.resources_used().gas.l1_gas,
                                    cairo_steps = receipt.resources_used().computation_resources.n_steps,
                                    "Transaction executed successfully"
                                );
                            }
                        }
                        Ok(ExecutionResult::Failed { error }) => {
                            error!(
                                target: LOG_TARGET,
                                tx_hash = format!("{:#x}", tx_hash),
                                error = %error,
                                "Transaction execution failed"
                            );
                        }
                        Err(e) => {
                            error!(
                                target: LOG_TARGET,
                                tx_hash = format!("{:#x}", tx_hash),
                                error = %e,
                                "Error executing transaction"
                            );
                        }
                    }

                    // Continue the loop to process the next transaction
                    continue;
                }

                Poll::Ready(None) => {
                    // Stream has ended (pool was dropped)
                    info!(target: LOG_TARGET, "Transaction stream ended");
                    return Poll::Ready(());
                }

                Poll::Pending => {
                    // Stream is exhausted - no more transactions available right now.
                    // Yield control back to the executor until we're polled again.
                    return Poll::Pending;
                }
            }
        }
    }
}

// Tests are intentionally omitted as they would require a full backend setup with
// blockchain state. Integration tests should be written separately to properly test
// the optimistic executor with a real backend instance.
