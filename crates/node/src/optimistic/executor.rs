use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::stream::StreamExt;
use katana_core::backend::storage::Blockchain;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::ExecutorFactory;
use katana_pool::ordering::FiFo;
use katana_pool::{PendingTransactions, PoolTransaction, TransactionPool};
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::providers::db::cached::CachedDbProvider;
use katana_rpc_types::BroadcastedTxWithChainId;
use katana_tasks::{JoinHandle, TaskSpawner};
use tracing::{debug, error, info, trace};

use crate::optimistic::pool::TxPool;

const LOG_TARGET: &str = "optimistic_executor";

#[derive(Debug)]
pub struct OptimisticExecutor {
    pool: TxPool,
    optimistic_state: CachedDbProvider<katana_db::Db>,
    executor_factory: Arc<BlockifierFactory>,
    storage: Blockchain,
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
        storage: Blockchain,
        optimistic_state: CachedDbProvider<katana_db::Db>,
        executor_factory: Arc<BlockifierFactory>,
        task_spawner: TaskSpawner,
    ) -> Self {
        Self { pool, optimistic_state, executor_factory, task_spawner, storage }
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
        self.task_spawner.build_task().name("Optimistic Executor").spawn(
            OptimisticExecutorActor::new(
                self.pool,
                self.storage,
                self.optimistic_state,
                self.executor_factory,
            ),
        )
    }
}

#[derive(Debug)]
struct OptimisticExecutorActor {
    pool: TxPool,
    optimistic_state: CachedDbProvider<katana_db::Db>,
    /// Stream of pending transactions from the pool
    pending_txs: PendingTransactions<BroadcastedTxWithChainId, FiFo<BroadcastedTxWithChainId>>,
    storage: Blockchain,
    executor_factory: Arc<BlockifierFactory>,
}

impl OptimisticExecutorActor {
    /// Creates a new executor actor with the given pending transactions stream.
    fn new(
        pool: TxPool,
        storage: Blockchain,
        optimistic_state: CachedDbProvider<katana_db::Db>,
        executor_factory: Arc<BlockifierFactory>,
    ) -> Self {
        let pending_txs = pool.pending_transactions();
        Self { pool, optimistic_state, pending_txs, storage, executor_factory }
    }

    /// Execute a single transaction optimistically against the latest state.
    fn execute_transaction(&self, tx: BroadcastedTxWithChainId) -> anyhow::Result<()> {
        let latest_state = self.optimistic_state.latest().unwrap();
        let mut executor = self.executor_factory.with_state(latest_state);

        // Execute the transaction
        let tx_hash = tx.hash();

        let _ = executor.execute_transactions(vec![tx.into()]).unwrap();

        let output = executor.take_execution_output().unwrap();
        self.optimistic_state.merge_state_updates(&output.states);
        self.pool.remove_transactions(&[tx_hash]);

        Ok(())
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

                    let tx_hash = tx.hash();
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
                        Ok(()) => {}
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
