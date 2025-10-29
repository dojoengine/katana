use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::stream::StreamExt;
use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{ExecutionResult, ExecutorFactory};
use katana_pool::{PendingTransactions, PoolOrd, PoolTransaction, TransactionPool, TxPool};
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_provider::api::state::StateFactoryProvider;
use katana_tasks::{JoinHandle, TaskSpawner};
use tracing::{debug, error, info, trace, warn};

const LOG_TARGET: &str = "optimistic_executor";

/// The `OptimisticExecutor` is an actor-based component that listens to incoming transactions
/// from the pool and executes them optimistically as they arrive.
///
/// This component subscribes to the pool's pending transaction stream and processes each
/// transaction as soon as it's available, without waiting for block production.
#[allow(missing_debug_implementations)]
pub struct OptimisticExecutor {
    /// The transaction pool to subscribe to
    pool: TxPool,
    /// The backend containing the executor factory and blockchain state
    backend: Arc<Backend<BlockifierFactory>>,
    /// Task spawner for running the executor actor
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
        backend: Arc<Backend<BlockifierFactory>>,
        task_spawner: TaskSpawner,
    ) -> Self {
        Self { pool, backend, task_spawner }
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
        info!(target: LOG_TARGET, "Starting optimistic executor");

        let pending_txs = self.pool.pending_transactions();
        let actor = OptimisticExecutorActor::new(pending_txs, self.backend);

        self.task_spawner.build_task().name("Optimistic Executor").spawn(actor)
    }
}

/// The internal actor that processes transactions from the pending transactions stream.
#[allow(missing_debug_implementations)]
struct OptimisticExecutorActor<O>
where
    O: PoolOrd<Transaction = ExecutableTxWithHash>,
{
    /// Stream of pending transactions from the pool
    pending_txs: PendingTransactions<ExecutableTxWithHash, O>,
    /// The backend for executing transactions
    backend: Arc<Backend<BlockifierFactory>>,
}

impl<O> OptimisticExecutorActor<O>
where
    O: PoolOrd<Transaction = ExecutableTxWithHash>,
{
    /// Creates a new executor actor with the given pending transactions stream.
    fn new(
        pending_txs: PendingTransactions<ExecutableTxWithHash, O>,
        backend: Arc<Backend<BlockifierFactory>>,
    ) -> Self {
        Self { pending_txs, backend }
    }

    /// Execute a single transaction optimistically against the latest state.
    fn execute_transaction(&self, tx: ExecutableTxWithHash) -> Result<ExecutionResult, String> {
        let provider = self.backend.blockchain.provider();

        // Get the latest state to execute against
        let latest_state =
            provider.latest().map_err(|e| format!("Failed to get latest state: {e}"))?;

        // Create an executor with the latest state
        let mut executor = self.backend.executor_factory.with_state(latest_state);

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
            }
            Err(e) => Err(format!("Execution failed: {e}")),
        }
    }
}

impl<O> Future for OptimisticExecutorActor<O>
where
    O: PoolOrd<Transaction = ExecutableTxWithHash>,
{
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
