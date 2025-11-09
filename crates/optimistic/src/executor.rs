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
use katana_primitives::block::{BlockIdOrTag, GasPrices};
use katana_primitives::env::BlockEnv;
use katana_primitives::transaction::TxWithHash;
use katana_primitives::version::StarknetVersion;
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::providers::db::cached::{CachedStateProvider, SharedStateCache};
use katana_rpc_client::starknet::Client;
use katana_rpc_types::block::GetBlockWithTxHashesResponse;
use katana_rpc_types::BroadcastedTxWithChainId;
use katana_tasks::{CpuBlockingJoinHandle, JoinHandle, Result as TaskResult, TaskSpawner};
use parking_lot::RwLock;
use tokio::time::sleep;
use tracing::{debug, error, info, trace};

use crate::pool::TxPool;

const LOG_TARGET: &str = "optimistic";

#[derive(Debug, Clone)]
pub struct OptimisticState {
    pub state: SharedStateCache,
    pub transactions: Arc<RwLock<Vec<(TxWithHash, ExecutionResult)>>>,
}

impl OptimisticState {
    pub fn new() -> Self {
        Self { state: SharedStateCache::default(), transactions: Arc::new(RwLock::new(Vec::new())) }
    }

    pub fn get_optimistic_state(&self, base: Box<dyn StateProvider>) -> Box<dyn StateProvider> {
        Box::new(CachedStateProvider::new(base, self.state.clone()))
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
    block_env: Arc<RwLock<BlockEnv>>,
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
    /// * `block_env` - The initial block environment
    pub fn new(
        pool: TxPool,
        storage: Blockchain,
        optimistic_state: OptimisticState,
        executor_factory: Arc<BlockifierFactory>,
        task_spawner: TaskSpawner,
        client: Client,
        block_env: BlockEnv,
    ) -> Self {
        Self {
            pool,
            optimistic_state,
            executor_factory,
            task_spawner,
            storage,
            client,
            block_env: Arc::new(RwLock::new(block_env)),
        }
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
                self.storage.clone(),
                self.optimistic_state.clone(),
                self.executor_factory,
                self.task_spawner.clone(),
                self.block_env.clone(),
            ),
        );

        // Spawn the block polling task
        let client = self.client;
        let optimistic_state = self.optimistic_state;
        let block_env = self.block_env;
        // self.task_spawner.build_task().name("Block Polling").spawn(async move {
        //     Self::poll_confirmed_blocks(client, optimistic_state, block_env).await;
        // });

        executor_handle
    }

    /// Polls for confirmed blocks every 2 seconds and removes transactions from the optimistic
    /// state when they appear in confirmed blocks. Also updates the block environment.
    async fn poll_confirmed_blocks(
        client: Client,
        optimistic_state: OptimisticState,
        block_env: Arc<RwLock<BlockEnv>>,
    ) {
        let mut last_block_number = None;

        loop {
            sleep(Duration::from_secs(5)).await;

            match client.get_block_with_tx_hashes(BlockIdOrTag::Latest).await {
                Ok(block_response) => {
                    let (block_number, block_tx_hashes, new_block_env) = match &block_response {
                        GetBlockWithTxHashesResponse::Block(block) => {
                            let env = BlockEnv {
                                number: block.block_number,
                                timestamp: block.timestamp,
                                l2_gas_prices: GasPrices {
                                    eth: block.l2_gas_price.price_in_wei.try_into().unwrap(),
                                    strk: block.l2_gas_price.price_in_fri.try_into().unwrap(),
                                },
                                l1_gas_prices: GasPrices {
                                    eth: block.l1_gas_price.price_in_wei.try_into().unwrap(),
                                    strk: block.l1_gas_price.price_in_fri.try_into().unwrap(),
                                },
                                l1_data_gas_prices: GasPrices {
                                    eth: block.l1_data_gas_price.price_in_wei.try_into().unwrap(),
                                    strk: block.l1_data_gas_price.price_in_fri.try_into().unwrap(),
                                },
                                sequencer_address: block.sequencer_address,
                                starknet_version: StarknetVersion::parse(&block.starknet_version)
                                    .unwrap_or_default(),
                            };
                            (block.block_number, block.transactions.clone(), env)
                        }
                        GetBlockWithTxHashesResponse::PreConfirmed(block) => {
                            let env = BlockEnv {
                                number: block.block_number,
                                timestamp: block.timestamp,
                                l2_gas_prices: GasPrices {
                                    eth: block.l2_gas_price.price_in_wei.try_into().unwrap(),
                                    strk: block.l2_gas_price.price_in_fri.try_into().unwrap(),
                                },
                                l1_gas_prices: GasPrices {
                                    eth: block.l1_gas_price.price_in_wei.try_into().unwrap(),
                                    strk: block.l1_gas_price.price_in_fri.try_into().unwrap(),
                                },
                                l1_data_gas_prices: GasPrices {
                                    eth: block.l1_data_gas_price.price_in_wei.try_into().unwrap(),
                                    strk: block.l1_data_gas_price.price_in_fri.try_into().unwrap(),
                                },
                                sequencer_address: block.sequencer_address,
                                starknet_version: StarknetVersion::parse(&block.starknet_version)
                                    .unwrap_or_default(),
                            };
                            (block.block_number, block.transactions.clone(), env)
                        }
                    };

                    // Check if this is a new block
                    if let Some(last_num) = last_block_number {
                        if block_number <= last_num {
                            // Same block, skip processing
                            continue;
                        }
                    }

                    // Update the last seen block number
                    last_block_number = Some(block_number);
                    debug!(target: LOG_TARGET, %block_number, "New block received.");

                    // Update the block environment for the next optimistic execution
                    *block_env.write() = new_block_env;
                    trace!(target: LOG_TARGET, block_number, "Updated block environment");

                    if block_tx_hashes.is_empty() {
                        continue;
                    }

                    // Get the current optimistic transactions
                    let mut optimistic_txs = optimistic_state.transactions.write();

                    // Filter out transactions that are confirmed in this block
                    let initial_count = optimistic_txs.len();
                    optimistic_txs.retain(|(tx, _)| !block_tx_hashes.contains(&tx.hash));

                    let removed_count = initial_count - optimistic_txs.len();
                    if removed_count > 0 {
                        debug!(
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
    block_env: Arc<RwLock<BlockEnv>>,
}

impl OptimisticExecutorActor {
    /// Creates a new executor actor with the given pending transactions stream.
    fn new(
        pool: TxPool,
        storage: Blockchain,
        optimistic_state: OptimisticState,
        executor_factory: Arc<BlockifierFactory>,
        task_spawner: TaskSpawner,
        block_env: Arc<RwLock<BlockEnv>>,
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
            block_env,
        }
    }

    /// Execute a single transaction optimistically against the latest state.
    fn execute_transaction(
        pool: TxPool,
        storage: Blockchain,
        optimistic_state: OptimisticState,
        executor_factory: Arc<BlockifierFactory>,
        block_env: Arc<RwLock<BlockEnv>>,
        tx: BroadcastedTxWithChainId,
    ) -> anyhow::Result<()> {
        let latest_state = storage.provider().latest()?;
        let state = optimistic_state.get_optimistic_state(latest_state);

        // Get the current block environment
        let current_block_env = block_env.read().clone();

        let mut executor = executor_factory.with_state_and_block_env(state, current_block_env);

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

                    // Spawn the transaction execution on the blocking CPU pool
                    let pool = this.pool.clone();
                    let storage = this.storage.clone();
                    let optimistic_state = this.optimistic_state.clone();
                    let executor_factory = this.executor_factory.clone();
                    let block_env = this.block_env.clone();

                    let execution_future = this.task_spawner.cpu_bound().spawn(move || {
                        Self::execute_transaction(
                            pool,
                            storage,
                            optimistic_state,
                            executor_factory,
                            block_env,
                            tx,
                        )
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
