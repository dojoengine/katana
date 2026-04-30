use std::{sync::Arc, time::Duration};

use anyhow::Result;
use starknet::{
    core::types::BlockId,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
};
use tokio::{
    sync::{
        mpsc::{self, Sender},
        Mutex,
    },
    task,
    time::sleep,
};
use tracing::{debug, error, trace};
use url::Url;

use crate::{
    block_ingestor::{
        BatchingBlockIngestorBuilder, BlockInfo, BlockIngestor, BlockIngestorBuilder,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::{BlockStatus, PersistantStorage},
};

const BLOCK_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const TASK_BUFFER_SIZE: usize = 4;
const MAX_RETRIES: usize = 3;

/// A block ingestor which collects new blocks by polling a Starknet RPC endpoint.
///
/// Responsibilities:
/// - Track the current block and advance it as the chain progresses.
/// - Re-queue blocks that previously failed.
/// - Fetch the `StateUpdate` for each block and store it in the DB.
/// - Emit `BlockInfo { status: Mined }` downstream for further processing.
///
/// PIE generation is intentionally **not** done here. It is the responsibility
/// of the next pipeline stage (e.g. `SnosPieGenerator`).
#[derive(Debug)]
pub struct PollingBlockIngestor<DB> {
    rpc_url: Url,
    current_block: u64,
    channel: Sender<BlockInfo>,
    finish_handle: FinishHandle,
    db: DB,
    workers_count: usize,
}

#[derive(Debug)]
pub struct PollingBlockIngestorBuilder<DB> {
    rpc_url: Url,
    start_block: Option<u64>,
    channel: Option<Sender<BlockInfo>>,
    db: DB,
    workers_count: usize,
}

impl<DB> PollingBlockIngestor<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn get_latest_block(&self) -> Option<u64> {
        let provider = JsonRpcClient::new(HttpTransport::new(self.rpc_url.clone()));

        let block_number = crate::utils::retry_with_backoff(
            || provider.block_number(),
            "get_latest_block",
            MAX_RETRIES as u32,
            Duration::from_secs(5),
        )
        .await;

        match block_number {
            Ok(block_number) => Some(block_number),
            Err(err) => {
                error!(error = ?err, "Failed to fetch latest block");
                None
            }
        }
    }

    /// Worker function: fetches the state update for a block and emits `BlockInfo { status: Mined }`.
    async fn worker(
        task_rx: Arc<Mutex<mpsc::Receiver<u64>>>,
        finish_handle: FinishHandle,
        rpc_url: Url,
        channel: mpsc::Sender<BlockInfo>,
        db: DB,
    ) where
        DB: PersistantStorage + Send + Sync + 'static,
    {
        loop {
            let block_number = if let Some(n) = task_rx.lock().await.recv().await {
                n
            } else {
                break;
            };

            if finish_handle.is_shutdown_requested() {
                break;
            }

            db.initialize_block(block_number.try_into().unwrap())
                .await
                .unwrap();

            let state_update = &JsonRpcClient::new(HttpTransport::new(rpc_url.clone()))
                .get_state_update(BlockId::Number(block_number))
                .await
                .unwrap();

            let state_update = match state_update {
                starknet::core::types::MaybePreConfirmedStateUpdate::Update(u) => u,
                starknet::core::types::MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
                    panic!("PreConfirmedStateUpdate not supported")
                }
            };

            db.add_state_update(block_number.try_into().unwrap(), state_update.clone())
                .await
                .unwrap();

            trace!(block_number, "Block mined, forwarding downstream");

            let new_block = BlockInfo {
                number: block_number,
                status: BlockStatus::Mined,
                state_update: Some(state_update.clone()),
            };

            if channel.send(new_block).await.is_err() {
                error!(block_number, "Failed to send block");
            }
        }
    }

    /// Continuously fetches the latest available block and sends it to the worker queue.
    async fn run(mut self) {
        let (task_tx, task_rx) = mpsc::channel(TASK_BUFFER_SIZE);
        let mut workers = Vec::new();
        let task_rx = Arc::new(Mutex::new(task_rx));

        for _ in 0..self.workers_count {
            let worker_task_rx = task_rx.clone();
            let finish_handle = self.finish_handle.clone();
            let rpc_url = self.rpc_url.clone();
            let channel = self.channel.clone();

            workers.push(task::spawn(Self::worker(
                worker_task_rx,
                finish_handle,
                rpc_url,
                channel,
                self.db.clone(),
            )));
        }

        while !self.finish_handle.is_shutdown_requested() {
            match self.get_latest_block().await {
                Some(latest_block) if latest_block >= self.current_block => {
                    if let Ok(mut failed_blocks) = self.db.get_failed_blocks().await {
                        let block_ids: Vec<u32> = failed_blocks.iter().map(|(id, _)| *id).collect();
                        for (block_id, _) in failed_blocks.drain(..) {
                            if task_tx.send(block_id as u64).await.is_err() {
                                return;
                            }
                        }
                        self.db
                            .mark_failed_blocks_as_handled(&block_ids)
                            .await
                            .unwrap();
                    }
                    if task_tx.send(self.current_block).await.is_err() {
                        return;
                    }
                    self.current_block += 1;
                }
                _ => {
                    sleep(BLOCK_CHECK_INTERVAL).await;
                }
            }
        }

        drop(task_tx);
        futures_util::future::join_all(workers).await;
        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<DB> PollingBlockIngestorBuilder<DB> {
    pub fn new(rpc_url: Url, db: DB, workers_count: usize) -> Self {
        Self {
            rpc_url,
            start_block: None,
            channel: None,
            db,
            workers_count,
        }
    }
}

impl<DB> BlockIngestorBuilder for PollingBlockIngestorBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Ingestor = PollingBlockIngestor<DB>;

    fn build(self) -> Result<Self::Ingestor> {
        Ok(PollingBlockIngestor {
            rpc_url: self.rpc_url,
            db: self.db,
            current_block: self
                .start_block
                .ok_or_else(|| anyhow::anyhow!("`start_block` not set"))?,
            channel: self
                .channel
                .ok_or_else(|| anyhow::anyhow!("`channel` not set"))?,
            finish_handle: FinishHandle::new(),
            workers_count: self.workers_count,
        })
    }

    fn start_block(mut self, start_block: u64) -> Self {
        self.start_block = Some(start_block);
        self
    }

    fn channel(mut self, channel: Sender<BlockInfo>) -> Self {
        self.channel = Some(channel);
        self
    }
}

impl<DB> BlockIngestor for PollingBlockIngestor<DB> where
    DB: PersistantStorage + Send + Sync + Clone + 'static
{
}

impl<DB> Daemon for PollingBlockIngestor<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

// ── Batching variant ─────────────────────────────────────────────────────────

/// A block ingestor that polls a Starknet RPC endpoint and emits ordered batches of blocks.
///
/// Blocks are fetched sequentially, so batches are always contiguous and in order — no
/// downstream reordering stage is needed.  A batch is emitted when either:
/// - `batch_size` blocks have accumulated, or
/// - `idle_timeout` elapses without a new block becoming available on-chain.
#[derive(Debug)]
pub struct BatchingPollingBlockIngestor<DB> {
    rpc_url: Url,
    current_block: u64,
    channel: tokio::sync::mpsc::Sender<Vec<BlockInfo>>,
    finish_handle: FinishHandle,
    db: DB,
    batch_size: usize,
    idle_timeout: Duration,
}

#[derive(Debug)]
pub struct BatchingPollingBlockIngestorBuilder<DB> {
    rpc_url: Url,
    start_block: Option<u64>,
    channel: Option<tokio::sync::mpsc::Sender<Vec<BlockInfo>>>,
    db: DB,
    batch_size: usize,
    idle_timeout: Duration,
}

impl<DB> BatchingPollingBlockIngestorBuilder<DB> {
    pub fn new(rpc_url: Url, db: DB, batch_size: usize, idle_timeout: Duration) -> Self {
        Self {
            rpc_url,
            start_block: None,
            channel: None,
            db,
            batch_size,
            idle_timeout,
        }
    }
}

impl<DB> BatchingBlockIngestorBuilder for BatchingPollingBlockIngestorBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Ingestor = BatchingPollingBlockIngestor<DB>;

    fn build(self) -> Result<Self::Ingestor> {
        Ok(BatchingPollingBlockIngestor {
            rpc_url: self.rpc_url,
            db: self.db,
            current_block: self
                .start_block
                .ok_or_else(|| anyhow::anyhow!("`start_block` not set"))?,
            channel: self
                .channel
                .ok_or_else(|| anyhow::anyhow!("`channel` not set"))?,
            finish_handle: FinishHandle::new(),
            batch_size: self.batch_size,
            idle_timeout: self.idle_timeout,
        })
    }

    fn start_block(mut self, start_block: u64) -> Self {
        self.start_block = Some(start_block);
        self
    }

    fn channel(mut self, channel: tokio::sync::mpsc::Sender<Vec<BlockInfo>>) -> Self {
        self.channel = Some(channel);
        self
    }
}

impl<DB> BlockIngestor for BatchingPollingBlockIngestor<DB> where
    DB: PersistantStorage + Send + Sync + Clone + 'static
{
}

impl<DB> Daemon for BatchingPollingBlockIngestor<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl<DB> BatchingPollingBlockIngestor<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn get_latest_block(&self) -> Option<u64> {
        let provider = JsonRpcClient::new(HttpTransport::new(self.rpc_url.clone()));

        let block_number = crate::utils::retry_with_backoff(
            || provider.block_number(),
            "get_latest_block",
            MAX_RETRIES as u32,
            Duration::from_secs(5),
        )
        .await;

        match block_number {
            Ok(n) => Some(n),
            Err(err) => {
                error!(error = ?err, "Failed to fetch latest block");
                None
            }
        }
    }

    /// Fetches the state update for `block_number`, stores it in the DB, and returns a
    /// [`BlockInfo`].  Returns an error if the RPC call fails after retries.
    async fn fetch_block(&self, block_number: u64) -> Result<BlockInfo> {
        let provider = JsonRpcClient::new(HttpTransport::new(self.rpc_url.clone()));

        self.db.initialize_block(block_number.try_into()?).await?;

        let state_update = crate::utils::retry_with_backoff(
            || provider.get_state_update(BlockId::Number(block_number)),
            "get_state_update",
            MAX_RETRIES as u32,
            Duration::from_secs(5),
        )
        .await?;

        let state_update = match state_update {
            starknet::core::types::MaybePreConfirmedStateUpdate::Update(u) => u,
            starknet::core::types::MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
                anyhow::bail!("PreConfirmedStateUpdate not supported for block {block_number}")
            }
        };

        self.db
            .add_state_update(block_number.try_into()?, state_update.clone())
            .await?;

        trace!(block_number, "Block fetched, buffering for next batch");

        Ok(BlockInfo {
            number: block_number,
            status: BlockStatus::Mined,
            state_update: Some(state_update),
        })
    }

    async fn run(mut self) {
        let mut pending: Vec<BlockInfo> = Vec::new();
        let mut idle_deadline = tokio::time::Instant::now() + self.idle_timeout;

        'outer: loop {
            if self.finish_handle.is_shutdown_requested() {
                break;
            }

            // Re-queue blocks that failed in a previous run.
            match self.db.get_failed_blocks().await {
                Ok(failed_blocks) if !failed_blocks.is_empty() => {
                    let block_ids: Vec<u32> = failed_blocks.iter().map(|(id, _)| *id).collect();
                    for (block_id, _) in failed_blocks {
                        match self.fetch_block(block_id as u64).await {
                            Ok(info) => {
                                idle_deadline = tokio::time::Instant::now() + self.idle_timeout;
                                pending.push(info);
                                if pending.len() >= self.batch_size {
                                    let batch = std::mem::take(&mut pending);
                                    if self.channel.send(batch).await.is_err() {
                                        break 'outer;
                                    }
                                }
                            }
                            Err(e) => {
                                error!(block_id, error = %e, "Failed to re-fetch failed block");
                            }
                        }
                    }
                    if let Err(e) = self.db.mark_failed_blocks_as_handled(&block_ids).await {
                        error!(error = %e, "Failed to mark failed blocks as handled");
                    }
                }
                Ok(_) => {}
                Err(e) => error!(error = %e, "Failed to query failed blocks"),
            }

            // Advance to the next block in sequence.
            let latest = match self.get_latest_block().await {
                Some(n) => n,
                None => {
                    tokio::select! {
                        _ = self.finish_handle.shutdown_requested() => break 'outer,
                        _ = sleep(BLOCK_CHECK_INTERVAL) => {}
                    }
                    continue;
                }
            };

            if latest >= self.current_block {
                match self.fetch_block(self.current_block).await {
                    Ok(info) => {
                        idle_deadline = tokio::time::Instant::now() + self.idle_timeout;
                        pending.push(info);
                        self.current_block += 1;

                        if pending.len() >= self.batch_size {
                            let batch = std::mem::take(&mut pending);
                            if self.channel.send(batch).await.is_err() {
                                break 'outer;
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            block_number = self.current_block,
                            error = %e,
                            "Failed to fetch block, retrying"
                        );
                        tokio::select! {
                            _ = self.finish_handle.shutdown_requested() => break 'outer,
                            _ = sleep(BLOCK_CHECK_INTERVAL) => {}
                        }
                    }
                }
            } else {
                tokio::select! {
                    _ = self.finish_handle.shutdown_requested() => break 'outer,
                    _ = sleep(BLOCK_CHECK_INTERVAL) => {}
                    _ = tokio::time::sleep_until(idle_deadline) => {
                        if !pending.is_empty() {
                            debug!(count = pending.len(), "Idle timeout — flushing partial batch");
                            let batch = std::mem::take(&mut pending);
                            if self.channel.send(batch).await.is_err() {
                                break 'outer;
                            }
                        }
                        idle_deadline = tokio::time::Instant::now() + self.idle_timeout;
                    }
                }
            }
        }

        // Flush any blocks accumulated before shutdown.
        if !pending.is_empty() {
            let _ = self.channel.send(pending).await;
        }

        debug!("BatchingPollingBlockIngestor graceful shutdown finished");
        self.finish_handle.finish();
    }
}
