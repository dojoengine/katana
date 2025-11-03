use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::stream::StreamExt;
use futures::FutureExt;
use katana_core::backend::storage::Blockchain;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{ExecutionResult, ExecutorFactory};
use katana_pool::ordering::FiFo;
use katana_pool::{PendingTransactions, PoolTransaction, TransactionPool};
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::transaction::TxWithHash;
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::providers::db::cached::CachedDbProvider;
use katana_rpc_client::starknet::Client;
use katana_rpc_types::BroadcastedTxWithChainId;
use katana_tasks::{CpuBlockingJoinHandle, JoinHandle, Result as TaskResult, TaskSpawner};
use parking_lot::RwLock;
use tokio::time::sleep;
use tracing::{debug, error, info, trace};

use crate::pool::TxPool;

const LOG_TARGET: &str = "optimistic_executor";

#[derive(Debug, Clone)]
pub struct OptimisticState {
    pub state: CachedDbProvider<katana_db::Db>,
    pub transactions: Arc<RwLock<Vec<(TxWithHash, ExecutionResult)>>>,
}

impl OptimisticState {
    pub fn new(state: CachedDbProvider<katana_db::Db>) -> Self {
        Self { state, transactions: Arc::new(RwLock::new(Vec::new())) }
    }
}

#[derive(Debug)]
pub struct OptimisticExecutor {
    pool: TxPool,
    optimistic_state: OptimisticState,
    executor_factory: Arc<BlockifierFactory>,
    storage: Blockchain,
    task_spawner: TaskSpawner,
    client: Client,
}

impl OptimisticExecutor {
    /// Creates a new `OptimisticExecutor` instance.
    ///
    /// # Arguments
    ///
    /// * `pool` - The transaction pool to monitor for new transactions
    /// * `backend` - The backend containing the executor factory and blockchain state
    /// * `task_spawner` - The task spawner used to run the executor actor
    /// * `client` - The RPC client used to poll for confirmed blocks
    pub fn new(
        pool: TxPool,
        storage: Blockchain,
        optimistic_state: OptimisticState,
        executor_factory: Arc<BlockifierFactory>,
        task_spawner: TaskSpawner,
        client: Client,
    ) -> Self {
        Self { pool, optimistic_state, executor_factory, task_spawner, storage, client }
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
        // Spawn the transaction execution task
        let executor_handle = self.task_spawner.build_task().name("Optimistic Executor").spawn(
            OptimisticExecutorActor::new(
                self.pool,
                self.storage,
                self.optimistic_state.clone(),
                self.executor_factory,
                self.task_spawner.clone(),
            ),
        );

        // Spawn the block polling task
        let client = self.client;
        let optimistic_state = self.optimistic_state;
        self.task_spawner.build_task().name("Block Polling").spawn(async move {
            Self::poll_confirmed_blocks(client, optimistic_state).await;
        });

        executor_handle
    }

    /// Polls for confirmed blocks every 2 seconds and removes transactions from the optimistic
    /// state when they appear in confirmed blocks.
    async fn poll_confirmed_blocks(client: Client, optimistic_state: OptimisticState) {
        loop {
            sleep(Duration::from_secs(2)).await;

            match client.get_block_with_tx_hashes(BlockIdOrTag::Latest).await {
                Ok(block_response) => {
                    use katana_rpc_types::block::GetBlockWithTxHashesResponse;

                    let (block_number, block_tx_hashes) = match block_response {
                        GetBlockWithTxHashesResponse::Block(block) => {
                            (block.block_number, block.transactions)
                        }
                        GetBlockWithTxHashesResponse::PreConfirmed(block) => {
                            (block.block_number, block.transactions)
                        }
                    };

                    if block_tx_hashes.is_empty() {
                        continue;
                    }

                    trace!(
                        target: LOG_TARGET,
                        block_number = block_number,
                        tx_count = block_tx_hashes.len(),
                        "Polling confirmed block"
                    );

                    // Get the current optimistic transactions
                    let mut optimistic_txs = optimistic_state.transactions.write();

                    // Filter out transactions that are confirmed in this block
                    let initial_count = optimistic_txs.len();
                    optimistic_txs.retain(|(tx, _)| !block_tx_hashes.contains(&tx.hash));

                    let removed_count = initial_count - optimistic_txs.len();
                    if removed_count > 0 {
                        info!(
                            target: LOG_TARGET,
                            block_number = block_number,
                            removed_count = removed_count,
                            remaining_count = optimistic_txs.len(),
                            "Removed confirmed transactions from optimistic state"
                        );
                    }
                }
                Err(e) => {
                    error!(
                        target: LOG_TARGET,
                        error = %e,
                        "Error polling for confirmed blocks"
                    );
                }
            }
        }
    }
}

#[derive(Debug)]
struct OptimisticExecutorActor {
    pool: TxPool,
    optimistic_state: OptimisticState,
    pending_txs: PendingTransactions<BroadcastedTxWithChainId, FiFo<BroadcastedTxWithChainId>>,
    storage: Blockchain,
    executor_factory: Arc<BlockifierFactory>,
    task_spawner: TaskSpawner,
    ongoing_execution: Option<CpuBlockingJoinHandle<anyhow::Result<()>>>,
}

impl OptimisticExecutorActor {
    /// Creates a new executor actor with the given pending transactions stream.
    fn new(
        pool: TxPool,
        storage: Blockchain,
        optimistic_state: OptimisticState,
        executor_factory: Arc<BlockifierFactory>,
        task_spawner: TaskSpawner,
    ) -> Self {
        let pending_txs = pool.pending_transactions();
        Self {
            pool,
            optimistic_state,
            pending_txs,
            storage,
            executor_factory,
            task_spawner,
            ongoing_execution: None,
        }
    }

    /// Execute a single transaction optimistically against the latest state.
    fn execute_transaction(
        pool: TxPool,
        optimistic_state: OptimisticState,
        executor_factory: Arc<BlockifierFactory>,
        tx: BroadcastedTxWithChainId,
    ) -> anyhow::Result<()> {
        let latest_state = optimistic_state.state.latest().unwrap();
        let mut executor = executor_factory.with_state(latest_state);

        // Execute the transaction
        let tx_hash = tx.hash();

        let _ = executor.execute_transactions(vec![tx.into()]).unwrap();

        let output = executor.take_execution_output().unwrap();
        optimistic_state.state.merge_state_updates(&output.states);

        // Add the executed transactions to the optimistic state
        for (tx, result) in output.transactions {
            optimistic_state.transactions.write().push((tx, result));
        }

        pool.remove_transactions(&[tx_hash]);

        Ok(())
    }
}

impl Future for OptimisticExecutorActor {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            // First, poll any ongoing execution to completion before processing new transactions
            if let Some(mut execution) = this.ongoing_execution.take() {
                match execution.poll_unpin(cx) {
                    Poll::Ready(result) => {
                        match result {
                            TaskResult::Ok(Ok(())) => {
                                // Execution completed successfully, continue to next transaction
                                trace!(target: LOG_TARGET, "Transaction execution completed successfully");
                            }
                            TaskResult::Ok(Err(e)) => {
                                error!(
                                    target: LOG_TARGET,
                                    error = %e,
                                    "Error executing transaction"
                                );
                            }
                            TaskResult::Err(e) => {
                                if e.is_cancelled() {
                                    error!(target: LOG_TARGET, "Transaction execution task cancelled");
                                } else {
                                    std::panic::resume_unwind(e.into_panic());
                                }
                            }
                        }
                        // Continue to process next transaction
                    }
                    Poll::Pending => {
                        // Execution is still ongoing, restore it and yield
                        this.ongoing_execution = Some(execution);
                        return Poll::Pending;
                    }
                }
            }

            // Process new transactions from the stream
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
                        "Spawning transaction execution on blocking pool"
                    );

                    // Spawn the transaction execution on the blocking CPU pool
                    let pool = this.pool.clone();
                    let optimistic_state = this.optimistic_state.clone();
                    let executor_factory = this.executor_factory.clone();

                    let execution_future = this.task_spawner.cpu_bound().spawn(move || {
                        Self::execute_transaction(pool, optimistic_state, executor_factory, tx)
                    });

                    this.ongoing_execution = Some(execution_future);

                    // Wake the task to poll the execution immediately
                    cx.waker().wake_by_ref();

                    // Continue the loop to poll the execution
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
