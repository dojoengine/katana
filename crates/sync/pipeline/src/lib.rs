#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Stage-based blockchain synchronization pipeline.
//!
//! This module provides a [`Pipeline`] for executing multiple [`Stage`]s sequentially to
//! synchronize blockchain data. The pipeline processes blocks in configurable chunks and can be
//! controlled via a [`PipelineHandle`].
//!
//! # Architecture
//!
//! The pipeline follows the [staged sync] architecture inspired by the [Erigon] Ethereum client.
//! Rather than performing all synchronization tasks concurrently, the sync process is decomposed
//! into distinct stages that execute sequentially:
//!
//! - **Sequential Execution**: Stages run one after another in a defined order, with each stage
//!   completing its work before the next stage begins.
//!
//! - **Isolation**: Each stage focuses on a specific aspect of synchronization (e.g., downloading
//!   block headers, downloading bodies, executing transactions, computing state). This separation
//!   makes each stage easier to understand, profile, and optimize independently.
//!
//! - **Checkpointing**: The pipeline tracks progress through checkpoints. Each stage maintains its
//!   own checkpoint, allowing the pipeline to resume from where it left off if interrupted.
//!
//! - **Chunked Processing**: Blocks are processed in configurable chunks, allowing for controlled
//!   progress and efficient resource usage.
//!
//! # Example
//!
//! ```no_run
//! use katana_pipeline::Pipeline;
//! use katana_provider::DbProviderFactory;
//! use katana_stage::Stage;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a provider factory for stage checkpoint management
//! let provider = DbProviderFactory::new_in_memory();
//!
//! // Create a pipeline with a chunk size of 100 blocks
//! let (mut pipeline, handle) = Pipeline::new(provider, 100);
//!
//! // Add stages to the pipeline (executed in order)
//! // pipeline.add_stage(MyDownloadStage::new());
//! // pipeline.add_stage(MyExecutionStage::new());
//!
//! // Subscribe to block notifications to monitor sync progress
//! let mut block_subscription = handle.subscribe_blocks();
//!
//! // Spawn a task to monitor synced blocks
//! tokio::spawn(async move {
//!     while let Ok(Some(block_num)) = block_subscription.changed().await {
//!         println!("Pipeline synced up to block: {}", block_num);
//!     }
//! });
//!
//! // Spawn the pipeline in a background task
//! let pipeline_task = tokio::spawn(async move { pipeline.run().await });
//!
//! // Set the target tip block to sync to
//! handle.set_tip(1000);
//!
//! // Later, update the tip as new blocks arrive
//! handle.set_tip(2000);
//!
//! // Stop the pipeline gracefully when done
//! handle.stop();
//!
//! // Wait for the pipeline to finish
//! pipeline_task.await??;
//! # Ok(())
//! # }
//! ```
//!
//! [staged sync]: https://ledgerwatch.github.io/turbo_geth_release.html#Staged-sync
//! [Erigon]: https://github.com/erigontech/erigon

use core::future::IntoFuture;

use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use katana_provider_api::stage::StageCheckpointProvider;
use katana_provider_api::ProviderError;
use katana_stage::{PruneInput, PruneOutput, Stage, StageExecutionInput, StageExecutionOutput};
use tokio::sync::watch::{self};
use tokio::task::yield_now;
use tracing::{debug, error, info, info_span, Instrument};

pub mod metrics;
pub use metrics::PipelineMetrics;

/// The result of a pipeline execution.
pub type PipelineResult<T> = Result<T, Error>;

/// The future type for [Pipeline]'s implementation of [IntoFuture].
pub type PipelineFut = BoxFuture<'static, PipelineResult<()>>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("stage not found: {id}")]
    StageNotFound { id: String },

    #[error("stage {id} execution failed: {error}")]
    StageExecution { id: &'static str, error: katana_stage::Error },

    #[error("stage {id} pruning failed: {error}")]
    StagePruning { id: &'static str, error: katana_stage::Error },

    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error("command channel closed")]
    CommandChannelClosed,
}

/// Commands that can be sent to control the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineCommand {
    /// Set the target tip block for the pipeline to sync to.
    SetTip(BlockNumber),
    /// Signal the pipeline to stop.
    Stop,
}

/// A subscription to pipeline block updates.
///
/// This subscription receives notifications whenever the pipeline completes processing
/// a block through all stages. The block number represents the highest block that has
/// been successfully processed by all pipeline stages for a given batch.
#[derive(Clone)]
pub struct PipelineBlockSubscription {
    rx: watch::Receiver<Option<BlockNumber>>,
}

impl PipelineBlockSubscription {
    /// Get the current processed block number, if any.
    ///
    /// Returns `None` if no blocks have been processed yet.
    pub fn block(&self) -> Option<BlockNumber> {
        *self.rx.borrow()
    }

    /// Wait for the next block to be processed and return its number.
    ///
    /// This method waits for the pipeline to process a new block and returns the block number.
    /// If the pipeline has been dropped, this returns an error.
    pub async fn changed(&mut self) -> Result<Option<BlockNumber>, watch::error::RecvError> {
        self.rx.changed().await?;
        Ok(*self.rx.borrow_and_update())
    }
}

impl std::fmt::Debug for PipelineBlockSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockSubscription").field("current_block", &self.block()).finish()
    }
}

/// A handle for controlling a running pipeline.
///
/// This handle allows external code to update the target tip block that the pipeline
/// should sync to, or to stop the pipeline.
#[derive(Debug, Clone)]
pub struct PipelineHandle {
    tx: watch::Sender<Option<PipelineCommand>>,
    block_tx: watch::Sender<Option<BlockNumber>>,
}

impl PipelineHandle {
    /// Sets the target tip block for the pipeline to sync to.
    ///
    /// The pipeline will process all blocks up to and including this block number.
    /// This method will wake up the pipeline if it's currently waiting for a new command.
    ///
    /// # Panics
    ///
    /// Panics if the [`Pipeline`] has been dropped.
    pub fn set_tip(&self, tip: BlockNumber) {
        self.tx.send(Some(PipelineCommand::SetTip(tip))).expect("pipeline is no longer running");
    }

    /// Signals the pipeline to stop gracefully.
    ///
    /// This will cause the pipeline's [`run`](Pipeline::run) method to exit after completing
    /// the current chunk of work. The pipeline will finish processing any in-flight stages
    /// before shutting down.
    ///
    /// # Panics
    ///
    /// Panics if the [`Pipeline`] has been dropped.
    pub fn stop(&self) {
        let _ = self.tx.send(Some(PipelineCommand::Stop));
    }

    /// Wait until the [`Pipeline`] has stopped.
    pub async fn stopped(&self) {
        self.tx.closed().await;
    }

    /// Subscribes to block notifications from the pipeline.
    ///
    /// Returns a subscription that will be notified whenever the pipeline completes processing
    /// a block through all stages. The block number represents the highest block that has
    /// been successfully processed by all pipeline stages.
    ///
    /// The subscription initially contains `None` until the first block is processed.
    pub fn subscribe_blocks(&self) -> PipelineBlockSubscription {
        PipelineBlockSubscription { rx: self.block_tx.subscribe() }
    }
}

/// Configuration for pruning behavior in the pipeline.
#[derive(Debug, Clone, Default)]
pub struct PruningConfig {
    /// Distance from tip. Blocks older than `tip - distance` will be pruned.
    /// `None` means no pruning (archive mode).
    pub distance: Option<u64>,
}

impl PruningConfig {
    /// Creates a new pruning configuration with the specified distance.
    pub fn new(distance: Option<u64>) -> Self {
        Self { distance }
    }

    /// Returns whether pruning is enabled.
    pub fn is_enabled(&self) -> bool {
        self.distance.is_some()
    }
}

/// Syncing pipeline.
///
/// The pipeline drives the execution of stages, running each stage to completion in the order they
/// were added.
///
/// # Unwinding
///
/// Currently, the pipeline does not support unwinding or chain reorganizations. If a new tip is
/// set to a lower block number than the previous tip, stages will simply skip execution since
/// their checkpoints are already beyond the target block.
///
/// Proper unwinding support would require each stage to implement rollback logic to revert their
/// state to an earlier block. This is a significant feature that would need to be designed and
/// implemented across all stages.
pub struct Pipeline {
    chunk_size: u64,
    storage_provider: DbProviderFactory,
    stages: Vec<Box<dyn Stage>>,
    cmd_rx: watch::Receiver<Option<PipelineCommand>>,
    cmd_tx: watch::Sender<Option<PipelineCommand>>,
    block_tx: watch::Sender<Option<BlockNumber>>,
    tip: Option<BlockNumber>,
    metrics: PipelineMetrics,
    pruning_config: PruningConfig,
}

impl Pipeline {
    /// Creates a new empty pipeline.
    ///
    /// # Arguments
    ///
    /// * `provider` - The provider for accessing stage checkpoints
    /// * `chunk_size` - The maximum number of blocks to process in a single iteration
    ///
    /// # Returns
    ///
    /// A tuple containing the pipeline instance and a handle for controlling it.
    pub fn new(provider: DbProviderFactory, chunk_size: u64) -> (Self, PipelineHandle) {
        let (tx, rx) = watch::channel(None);
        let (block_tx, _block_rx) = watch::channel(None);
        let handle = PipelineHandle { tx: tx.clone(), block_tx: block_tx.clone() };
        let pipeline = Self {
            stages: Vec::new(),
            cmd_rx: rx,
            cmd_tx: tx,
            block_tx,
            storage_provider: provider,
            chunk_size,
            tip: None,
            metrics: PipelineMetrics::new(),
            pruning_config: PruningConfig::default(),
        };
        (pipeline, handle)
    }

    /// Sets the pruning configuration for the pipeline.
    ///
    /// This controls how and when historical state is pruned during synchronization.
    pub fn set_pruning_config(&mut self, config: PruningConfig) {
        self.pruning_config = config;
    }

    /// Returns the current pruning configuration.
    pub fn pruning_config(&self) -> &PruningConfig {
        &self.pruning_config
    }

    /// Adds a new stage to the end of the pipeline.
    ///
    /// Stages are executed in the order they are added.
    pub fn add_stage<S: Stage + 'static>(&mut self, stage: S) {
        self.stages.push(Box::new(stage));
    }

    /// Adds multiple stages to the pipeline.
    ///
    /// Stages are executed in the order they appear in the iterator.
    pub fn add_stages(&mut self, stages: impl IntoIterator<Item = Box<dyn Stage>>) {
        self.stages.extend(stages);
    }

    /// Returns a handle for controlling the pipeline.
    ///
    /// The handle can be used to set the target tip block for the pipeline to sync to or to
    /// stop the pipeline.
    pub fn handle(&self) -> PipelineHandle {
        PipelineHandle { tx: self.cmd_tx.clone(), block_tx: self.block_tx.clone() }
    }

    /// Returns a reference to the pipeline metrics.
    pub fn metrics(&self) -> &PipelineMetrics {
        &self.metrics
    }
}

impl Pipeline {
    /// Runs the pipeline continuously until signaled to stop.
    ///
    /// The pipeline processes each stage in chunks up until it reaches the current tip, then waits
    /// for the tip to be updated via the [`PipelineHandle::set_tip`] or until stopped via
    /// [`PipelineHandle::stop`].
    ///
    /// # Errors
    ///
    /// Returns an error if any stage execution fails or it an error occurs while reading the
    /// checkpoint.
    pub async fn run(&mut self) -> PipelineResult<()> {
        let mut command_rx = self.cmd_rx.clone();

        loop {
            tokio::select! {
                biased;

                changed = command_rx.wait_for(|c| matches!(c, &Some(PipelineCommand::Stop))) => {
                    if changed.is_err() {
                        break;
                    }

                    debug!(target: "pipeline", "Received stop command.");
                    break;
                }

                result = self.run_loop() => {
                    if let Err(error) = result {
                        error!(target: "pipeline", %error, "Pipeline returned an error.");
                    }
                }
            }
        }

        info!(target: "pipeline", "Pipeline shutting down.");

        Ok(())
    }

    /// Runs all stages in the pipeline up to the specified block number.
    ///
    /// Each stage is executed sequentially from its current checkpoint to the target block.
    /// Stages that have already processed up to or beyond the target block are skipped.
    ///
    /// # Arguments
    ///
    /// * `to` - The target block number to process up to (inclusive)
    ///
    /// # Returns
    ///
    /// The minimum of the last block numbers processed by all stages. This represents the
    /// lower bound for the range of block the pipeline has successfully processed in this single
    /// run (aggregated across all stages).
    ///
    /// # Errors
    ///
    /// Returns an error if any stage execution fails or if the pipeline fails to read the
    /// checkpoint.
    pub async fn execute(&mut self, to: BlockNumber) -> PipelineResult<BlockNumber> {
        if self.stages.is_empty() {
            return Ok(to);
        }

        // This is so that lagging stages (ie stage with a checkpoint that is less than the rest of
        // the stages) will be executed, in the next cycle of `run_to`, with a `to` value
        // whose range from the stages' next checkpoint is equal to the pipeline batch size.
        //
        // This can actually be done without the allocation, but this makes reasoning about the
        // code easier. The majority of the execution time will be spent in `stage.execute` anyway
        // so optimizing this doesn't yield significant improvements.
        let mut last_block_processed_list: Vec<BlockNumber> = Vec::with_capacity(self.stages.len());

        for stage in self.stages.iter_mut() {
            let id = stage.id();
            let stage_metrics = self.metrics.stage(id);

            // Get the checkpoint for the stage, otherwise default to block number 0
            let checkpoint = self.storage_provider.provider_mut().execution_checkpoint(id)?;

            let span = info_span!(target: "pipeline", "stage.execute", stage = %id, %to);
            let enter = span.entered();

            let from = if let Some(checkpoint) = checkpoint {
                debug!(target: "pipeline", %checkpoint, "Found checkpoint.");
                stage_metrics.set_checkpoint(checkpoint);

                // Skip the stage if the checkpoint is greater than or equal to the target block
                // number
                if checkpoint >= to {
                    info!(target: "pipeline", %checkpoint, "Skipping stage - target already reached.");
                    last_block_processed_list.push(checkpoint);
                    continue;
                }

                // plus 1 because the checkpoint is the last block processed, so we need to start
                // from the next block
                checkpoint + 1
            } else {
                stage_metrics.set_checkpoint(0);
                0
            };

            let input = StageExecutionInput::new(from, to);
            info!(target: "pipeline", %from, %to, "Executing stage.");

            let span = enter.exit();
            let _guard = stage_metrics.execution_started();
            let StageExecutionOutput { last_block_processed } = stage
                .execute(&input)
                .instrument(span.clone())
                .await
                .map_err(|error| Error::StageExecution { id, error })?;

            let _enter = span.enter();
            info!(target: "pipeline", %from, %to, "Stage execution completed.");

            // Record blocks processed by this stage in this execution
            let blocks_processed = last_block_processed.saturating_sub(from.saturating_sub(1));
            stage_metrics.record_blocks_processed(blocks_processed);

            let provider_mut = self.storage_provider.provider_mut();
            provider_mut.set_execution_checkpoint(id, last_block_processed)?;
            provider_mut.commit()?;

            stage_metrics.set_checkpoint(last_block_processed);
            last_block_processed_list.push(last_block_processed);
            info!(target: "pipeline", checkpoint = %last_block_processed, "New checkpoint set.");
        }

        Ok(last_block_processed_list.into_iter().min().unwrap_or(to))
    }

    /// Runs pruning on all stages.
    pub async fn prune(&mut self) -> PipelineResult<()> {
        if self.stages.is_empty() {
            return Ok(());
        }

        for stage in self.stages.iter_mut() {
            let id = stage.id();

            let span = info_span!(target: "pipeline", "stage.prune", stage = %id);
            let enter = span.entered();

            // Get execution checkpoint (tip for this stage) and prune checkpoint
            let execution_checkpoint =
                self.storage_provider.provider_mut().execution_checkpoint(id)?;
            let prune_checkpoint = self.storage_provider.provider_mut().prune_checkpoint(id)?;

            let Some(tip) = execution_checkpoint else {
                info!(target: "pipeline", "Skipping stage - no data to prune (no execution checkpoint).");
                continue;
            };

            let prune_input = PruneInput::new(tip, self.pruning_config.distance, prune_checkpoint);

            let Some(range) = prune_input.prune_range() else {
                info!(target: "pipeline", "Skipping stage - nothing to prune (already caught up).");
                continue;
            };

            info!(target: "pipeline", distance = ?self.pruning_config.distance, from = range.start, to = range.end, "Pruning stage.");

            let span_inner = enter.exit();
            let PruneOutput { pruned_count } = stage
                .prune(&prune_input)
                .instrument(span_inner.clone())
                .await
                .map_err(|error| Error::StagePruning { id, error })?;

            // Update prune checkpoint to the last pruned block (range.end - 1 since range is
            // exclusive)
            if range.end > 0 {
                let provider_mut = self.storage_provider.provider_mut();
                provider_mut.set_prune_checkpoint(id, range.end - 1)?;
                provider_mut.commit()?;
            }

            let _enter = span_inner.enter();
            info!(target: "pipeline", %pruned_count, "Stage pruning completed.");
        }

        Ok(())
    }

    /// Run the pipeline loop.
    async fn run_loop(&mut self) -> PipelineResult<()> {
        let mut current_chunk_tip = self.chunk_size;

        loop {
            // Process blocks if we have a tip
            if let Some(tip) = self.tip {
                let to = current_chunk_tip.min(tip);
                let iteration_start = std::time::Instant::now();

                let last_block_processed = self.execute(to).await?;
                self.metrics.set_sync_position(last_block_processed);

                let iteration_duration = iteration_start.elapsed().as_secs_f64();
                self.metrics.record_iteration_duration(iteration_duration);

                // Notify subscribers about the newly processed block
                let _ = self.block_tx.send(Some(last_block_processed));

                // Run pruning if enabled
                if self.pruning_config.is_enabled() {
                    self.prune().await?;
                }

                if last_block_processed >= tip {
                    info!(target: "pipeline", %tip, "Finished syncing until tip.");
                    self.tip = None;
                    current_chunk_tip = last_block_processed;
                } else {
                    current_chunk_tip = (last_block_processed + self.chunk_size).min(tip);
                }
            } else {
                info!(target: "pipeline", "Waiting to receive new tip.");
                self.cmd_rx.changed().await.map_err(|_| Error::CommandChannelClosed)?;

                if let Some(PipelineCommand::SetTip(new_tip)) = *self.cmd_rx.borrow_and_update() {
                    info!(target: "pipeline", tip = %new_tip, "A new tip has been set.");
                    self.tip = Some(new_tip);
                    self.metrics.set_sync_target(new_tip);
                }
            }

            yield_now().await;
        }
    }
}

impl IntoFuture for Pipeline {
    type Output = PipelineResult<()>;
    type IntoFuture = PipelineFut;

    fn into_future(mut self) -> Self::IntoFuture {
        Box::pin(async move {
            self.run().await.inspect_err(|error| {
                error!(target: "pipeline", %error, "Pipeline failed.");
            })
        })
    }
}

impl core::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pipeline")
            .field("command", &self.cmd_rx)
            .field("provider", &self.storage_provider)
            .field("chunk_size", &self.chunk_size)
            .field("pruning_config", &self.pruning_config)
            .field("stages", &self.stages.iter().map(|s| s.id()).collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}
