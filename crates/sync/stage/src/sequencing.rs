use std::fmt::Debug;
use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::Result;
use futures::future::{self, BoxFuture};
use katana_core::backend::Backend;
use katana_core::service::block_producer::{BlockProducer, BlockProductionError};
use katana_core::service::{BlockProductionTask, TransactionMiner};
use katana_messaging::{MessagingConfig, MessagingService, MessagingTask};
use katana_pool::api::TransactionPool;
use katana_pool::TxPool;
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};
use katana_tasks::{JoinHandle, TaskSpawner};
use tracing::error;

pub type SequencingFut = BoxFuture<'static, Result<()>>;

/// The sequencing stage is responsible for advancing the chain state.
#[allow(missing_debug_implementations)]
pub struct Sequencing<PF>
where
    PF: ProviderFactory,
{
    pool: TxPool,
    backend: Arc<Backend<PF>>,
    task_spawner: TaskSpawner,
    block_producer: BlockProducer<PF>,
    messaging_config: Option<MessagingConfig>,
}

impl<PF> Sequencing<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO + Debug,
    <PF as ProviderFactory>::ProviderMut: ProviderRW + Debug,
{
    pub fn new(
        pool: TxPool,
        backend: Arc<Backend<PF>>,
        task_spawner: TaskSpawner,
        block_producer: BlockProducer<PF>,
        messaging_config: Option<MessagingConfig>,
    ) -> Self {
        Self { pool, backend, task_spawner, block_producer, messaging_config }
    }

    async fn run_messaging(&self) -> Result<JoinHandle<()>> {
        if let Some(config) = &self.messaging_config {
            let config = config.clone();
            let pool = self.pool.clone();
            let chain_spec = self.backend.chain_spec.clone();

            let service = MessagingService::new(config, chain_spec, pool).await?;
            let task = MessagingTask::new(service);

            let handle = self.task_spawner.build_task().name("Messaging").spawn(task);
            Ok(handle)
        } else {
            let handle = self.task_spawner.build_task().spawn(future::pending::<()>());
            Ok(handle)
        }
    }

    fn run_block_production(&self) -> JoinHandle<Result<(), BlockProductionError>> {
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
            // Build the messaging and block production tasks.
            let messaging = self.run_messaging().await?;
            let block_production = self.run_block_production();

            // Neither of these tasks should complete as they are meant to be run forever,
            // but if either of them do complete, the sequencing stage should return.
            //
            // Select on the tasks completion to prevent the task from failing silently (if any).
            tokio::select! {
                res = messaging => {
                    error!(target: "sequencing", reason = ?res, "Messaging task finished unexpectedly.");
                },
                res = block_production => {
                    error!(target: "sequencing", reason = ?res, "Block production task finished unexpectedly.");
                }
            }

            Ok(())
        })
    }
}
