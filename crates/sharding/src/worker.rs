use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use katana_pool::TransactionPool;
use katana_primitives::env::BlockEnv;
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::ProviderFactory;
use tracing::{error, trace};

use crate::scheduler::Scheduler;
use crate::shard::{Shard, ShardState};

/// A worker that picks shards from the scheduler and executes their pending transactions.
///
/// Each worker runs on a dedicated OS thread and blocks on the scheduler's condvar
/// when no work is available.
pub struct Worker {
    id: usize,
    scheduler: Scheduler,
}

impl Worker {
    pub fn new(id: usize, scheduler: Scheduler) -> Self {
        Self { id, scheduler }
    }

    /// Run the worker loop. This method blocks the calling thread until shutdown.
    pub fn run(self) -> Result<()> {
        trace!(worker_id = self.id, "Worker started.");

        loop {
            // Worker-owned shutdown check
            if self.scheduler.is_shutdown() {
                trace!(worker_id = self.id, "Shard worker shutting down.");
                break;
            }

            // Try to get the next shard (blocks up to ~100ms)
            let shard = match self.scheduler.next_task() {
                Some(shard) => shard,
                None => continue,
            };

            shard.set_state(ShardState::Running);
            trace!(worker_id = self.id, shard_id = %shard.id, "Processing shard.");

            let start = Instant::now();
            let quantum = self.scheduler.time_quantum();

            loop {
                match shard.execute() {
                    Ok(()) => {
                        trace!(worker_id = self.id, shard_id = %shard.id, "Shard execution completed successfully.");
                    }
                    Err(error) => {
                        error!(worker_id = self.id, shard_id = %shard.id, %error, "Shard execution failed.");
                    }
                }

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

        Ok(())
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
