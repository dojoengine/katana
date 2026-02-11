use std::sync::Arc;
use std::thread;
use std::time::Instant;

use katana_pool::TransactionPool;
use katana_primitives::env::BlockEnv;
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::ProviderFactory;
use parking_lot::RwLock;
use tracing::{error, info, trace};

use super::scheduler::ShardScheduler;
use super::types::{Shard, ShardState};

/// A worker that picks shards from the scheduler and executes their pending transactions.
///
/// Each worker runs on a dedicated OS thread and blocks on the scheduler's condvar
/// when no work is available.
pub struct ShardWorker {
    id: usize,
    scheduler: ShardScheduler,
    block_env: Arc<RwLock<BlockEnv>>,
}

impl ShardWorker {
    pub fn new(id: usize, scheduler: ShardScheduler, block_env: Arc<RwLock<BlockEnv>>) -> Self {
        Self { id, scheduler, block_env }
    }

    /// Run the worker loop. This method blocks the calling thread until shutdown.
    pub fn run(self) {
        trace!(worker_id = self.id, "Shard worker started.");

        loop {
            // Block until there's a shard to process (or shutdown)
            let shard = match self.scheduler.next_task() {
                Some(shard) => shard,
                None => {
                    info!(worker_id = self.id, "Shard worker shutting down.");
                    return;
                }
            };

            shard.set_state(ShardState::Running);
            trace!(worker_id = self.id, shard_id = %shard.id, "Processing shard.");

            let start = Instant::now();
            let quantum = self.scheduler.time_quantum();

            loop {
                // Collect pending transactions from the shard's pool
                let txs = Self::collect_pending_transactions(&shard);
                if txs.is_empty() {
                    break;
                }

                let tx_hashes: Vec<_> = txs.iter().map(|tx| tx.hash).collect();
                let tx_count = txs.len();

                // Get current state and shared block env
                let block_env = self.block_env.read().clone();

                match self.execute(&shard, txs, &block_env) {
                    Ok(()) => {
                        trace!(
                            worker_id = self.id,
                            shard_id = %shard.id,
                            %tx_count,
                            "Executed and committed transactions."
                        );
                    }
                    Err(e) => {
                        error!(
                            worker_id = self.id,
                            shard_id = %shard.id,
                            error = %e,
                            "Failed to execute/commit transactions."
                        );
                    }
                }

                // Remove executed txs from pool
                shard.pool.remove_transactions(&tx_hashes);

                // Check time quantum
                if start.elapsed() >= quantum {
                    break;
                }
            }

            // Re-schedule if there are still pending txs, otherwise go idle.
            // We need to set it back to Idle first so `schedule` can transition it.
            if shard.pool.size() > 0 {
                shard.set_state(ShardState::Idle);
                self.scheduler.schedule(Arc::clone(&shard));
            } else {
                shard.set_state(ShardState::Idle);
            }
        }
    }

    /// Collect all currently pending transactions from the shard's pool (non-blocking snapshot).
    fn collect_pending_transactions(shard: &Shard) -> Vec<ExecutableTxWithHash> {
        let pending = shard.pool.pending_transactions();
        pending.all.map(|ptx| (*ptx.tx).clone()).collect()
    }

    /// Execute transactions against the shard's state and commit results to storage.
    fn execute(
        &self,
        shard: &Shard,
        txs: Vec<ExecutableTxWithHash>,
        block_env: &BlockEnv,
    ) -> anyhow::Result<()> {
        let state = shard.provider.provider().latest()?;

        let mut executor = shard.backend.executor_factory.executor(state, block_env.clone());

        executor.execute_transactions(txs)?;
        let output = executor.take_execution_output()?;

        shard.backend.do_mine_block(block_env, output)?;

        Ok(())
    }
}

/// Spawn a pool of shard workers on dedicated OS threads.
pub fn spawn_workers(
    count: usize,
    scheduler: ShardScheduler,
    block_env: Arc<RwLock<BlockEnv>>,
) -> Vec<thread::JoinHandle<()>> {
    (0..count)
        .map(|id| {
            let worker = ShardWorker::new(id, scheduler.clone(), block_env.clone());
            thread::Builder::new()
                .name(format!("shard-worker-{id}"))
                .spawn(move || worker.run())
                .expect("failed to spawn shard worker thread")
        })
        .collect()
}
