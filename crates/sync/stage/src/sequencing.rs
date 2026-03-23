use std::fmt::Debug;
use std::future::IntoFuture;

use anyhow::Result;
use futures::future::BoxFuture;
use katana_core::service::block_producer::{BlockProducer, BlockProductionError};
use katana_core::service::{BlockProductionTask, TransactionMiner};
use katana_pool::{TransactionPool, TxPool};
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};
use katana_tasks::TaskSpawner;
use tracing::error;

pub type SequencingFut = BoxFuture<'static, Result<()>>;

/// The sequencing stage is responsible for advancing the chain state.
#[allow(missing_debug_implementations)]
pub struct Sequencing<PF>
where
    PF: ProviderFactory,
{
    pool: TxPool,
    task_spawner: TaskSpawner,
    block_producer: BlockProducer<PF>,
}

impl<PF> Sequencing<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO + Debug,
    <PF as ProviderFactory>::ProviderMut: ProviderRW + Debug,
{
    pub fn new(
        pool: TxPool,
        task_spawner: TaskSpawner,
        block_producer: BlockProducer<PF>,
    ) -> Self {
        Self { pool, task_spawner, block_producer }
    }

    fn run_block_production(&self) -> katana_tasks::JoinHandle<Result<(), BlockProductionError>> {
        // Create a new transaction miner with a subscription to the pool's pending transactions.
        let miner = TransactionMiner::new(self.pool.pending_transactions());
        let block_producer = self.block_producer.clone();
        let service = BlockProductionTask::new(self.pool.clone(), miner, block_producer);
        self.task_spawner.build_task().name("Block production").spawn(service)
    }
}

impl<PF> IntoFuture for Sequencing<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO + Debug,
    <PF as ProviderFactory>::ProviderMut: ProviderRW + Debug,
{
    type Output = Result<()>;
    type IntoFuture = SequencingFut;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let block_production = self.run_block_production();

            // The block production task should run forever. If it completes, something went wrong.
            match block_production.await {
                Ok(res) => {
                    error!(target: "sequencing", reason = ?res, "Block production task finished unexpectedly.");
                }
                Err(e) => {
                    error!(target: "sequencing", reason = ?e, "Block production task panicked.");
                }
            }

            Ok(())
        })
    }
}
