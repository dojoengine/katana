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
//! use katana_provider::providers::in_memory::InMemoryProvider;
//! use katana_stage::Stage;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a provider for stage checkpoint management
//! let provider = InMemoryProvider::new();
//!
//! // Create a pipeline with a chunk size of 100 blocks
//! let (mut pipeline, handle) = Pipeline::new(provider, 100);
//!
//! // Add stages to the pipeline (executed in order)
//! // pipeline.add_stage(MyDownloadStage::new());
//! // pipeline.add_stage(MyExecutionStage::new());
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
use katana_provider_api::stage::StageCheckpointProvider;
use katana_provider_api::ProviderError;
use katana_stage::{Stage, StageExecutionInput, StageExecutionOutput};
use tokio::sync::watch;
use tokio::task::yield_now;
use tracing::{debug, error, info, info_span, Instrument};

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

    #[error(transparent)]
    Provider(#[from] ProviderError),
}

/// Commands that can be sent to control the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineCommand {
    /// Set the target tip block for the pipeline to sync to.
    SetTip(BlockNumber),
    /// Signal the pipeline to stop.
    Stop,
}

/// A handle for controlling a running pipeline.
///
/// This handle allows external code to update the target tip block that the pipeline
/// should sync to, or to stop the pipeline.
#[derive(Debug, Clone)]
pub struct PipelineHandle {
    tx: watch::Sender<Option<PipelineCommand>>,
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
pub struct Pipeline<P> {
    chunk_size: u64,
    provider: P,
    stages: Vec<Box<dyn Stage>>,
    command_rx: watch::Receiver<Option<PipelineCommand>>,
    command_tx: watch::Sender<Option<PipelineCommand>>,
    tip: Option<BlockNumber>,
}

impl<P> Pipeline<P> {
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
    pub fn new(provider: P, chunk_size: u64) -> (Self, PipelineHandle) {
        let (tx, rx) = watch::channel(None);
        let handle = PipelineHandle { tx: tx.clone() };
        let pipeline = Self {
            stages: Vec::new(),
            command_rx: rx,
            command_tx: tx,
            provider,
            chunk_size,
            tip: None,
        };
        (pipeline, handle)
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
        PipelineHandle { tx: self.command_tx.clone() }
    }
}

impl<P: StageCheckpointProvider> Pipeline<P> {
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
        let mut command_rx = self.command_rx.clone();

        loop {
            tokio::select! {
                biased;

                changed = command_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }

                    // Check if the handle has sent a signal
                    match *self.command_rx.borrow_and_update() {
                        Some(PipelineCommand::Stop) => {
                            debug!(target: "pipeline", "Received stop command.");
                            break;
                        }
                        Some(PipelineCommand::SetTip(new_tip)) => {
                            info!(target: "pipeline", tip = %new_tip, "A new tip has been set.");
                            self.tip = Some(new_tip);
                        }
                        None => {}
                    }
                }

                result = self.run_loop() => {
                    if let Err(error) = result {
                        error!(target: "pipeline", %error, "Pipeline finished due to error.");
                        break;
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
    pub async fn run_once(&mut self, to: BlockNumber) -> PipelineResult<BlockNumber> {
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

            // Get the checkpoint for the stage, otherwise default to block number 0
            let checkpoint = self.provider.checkpoint(id)?;

            let span = info_span!(target: "pipeline", "stage.execute", stage = %id, %to);
            let enter = span.entered();

            let from = if let Some(checkpoint) = checkpoint {
                debug!(target: "pipeline", %checkpoint, "Found checkpoint.");

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
                0
            };

            let input = StageExecutionInput::new(from, to);
            info!(target: "pipeline", %from, %to, "Executing stage.");

            let span = enter.exit();
            let StageExecutionOutput { last_block_processed } = stage
                .execute(&input)
                .instrument(span.clone())
                .await
                .map_err(|error| Error::StageExecution { id, error })?;

            let _enter = span.enter();
            info!(target: "pipeline", %from, %to, "Stage execution completed.");

            self.provider.set_checkpoint(id, last_block_processed)?;
            last_block_processed_list.push(last_block_processed);
            info!(target: "pipeline", checkpoint = %last_block_processed, "New checkpoint set.");
        }

        Ok(last_block_processed_list.into_iter().min().unwrap_or(to))
    }

    /// Run the pipeline loop.
    async fn run_loop(&mut self) -> PipelineResult<()> {
        let mut current_chunk_tip = self.chunk_size;

        loop {
            // Process blocks if we have a tip
            if let Some(tip) = self.tip {
                let to = current_chunk_tip.min(tip);
                let last_block_processed = self.run_once(to).await?;

                if last_block_processed >= tip {
                    info!(target: "pipeline", %tip, "Finished syncing until tip.");
                    self.tip = None;
                    current_chunk_tip = last_block_processed;
                } else {
                    current_chunk_tip = (last_block_processed + self.chunk_size).min(tip);
                }

                continue;
            }

            info!(target: "pipeline", "Waiting to receive new tip.");

            // block until a new tip is set
            self.command_rx
                .wait_for(|c| matches!(c, &Some(PipelineCommand::SetTip(_))))
                .await
                .expect("qed; channel closed");

            yield_now().await;
        }
    }
}

impl<P> IntoFuture for Pipeline<P>
where
    P: StageCheckpointProvider + 'static,
{
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

impl<P> core::fmt::Debug for Pipeline<P>
where
    P: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pipeline")
            .field("command", &self.command_rx)
            .field("provider", &self.provider)
            .field("chunk_size", &self.chunk_size)
            .field("stages", &self.stages.iter().map(|s| s.id()).collect::<Vec<_>>())
            .finish()
    }
}
