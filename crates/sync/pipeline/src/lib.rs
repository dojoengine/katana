#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use core::future::IntoFuture;

use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::api::ProviderError;
use katana_stage::{Stage, StageExecutionInput};
use tokio::sync::watch;
use tracing::{debug, error, info, trace};

/// The result of a pipeline execution.
pub type PipelineResult<T> = Result<T, Error>;

/// The future type for [Pipeline]'s implementation of [IntoFuture].
pub type PipelineFut = BoxFuture<'static, PipelineResult<()>>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Stage not found: {id}")]
    StageNotFound { id: String },

    #[error(transparent)]
    Stage(#[from] katana_stage::Error),

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
        info!(target: "pipeline", %tip, "Setting new tip");
        self.tx.send(Some(PipelineCommand::SetTip(tip))).expect("channel closed");
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
        info!(target: "pipeline", "Signaling pipeline to stop");
        self.tx.send(Some(PipelineCommand::Stop)).expect("channel closed");
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
        let pipeline =
            Self { stages: Vec::new(), command_rx: rx, command_tx: tx, provider, chunk_size };
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
    pub fn add_stages(&mut self, stages: impl Iterator<Item = Box<dyn Stage>>) {
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
        let mut current_chunk_tip = self.chunk_size;
        let mut current_tip: Option<BlockNumber> = None;

        loop {
            // Check if the handle has sent a signal
            match *self.command_rx.borrow_and_update() {
                Some(PipelineCommand::Stop) => {
                    debug!(target: "pipeline", "Received stop command.");
                    break;
                }
                Some(PipelineCommand::SetTip(tip)) => {
                    trace!(target: "pipeline", %tip, "Received new tip.");
                    current_tip = Some(tip);
                }
                None => {}
            }

            // Process blocks if we have a tip
            if let Some(tip) = current_tip {
                let to = current_chunk_tip.min(tip);
                let last_block_processed = self.run_to(to).await?;

                if last_block_processed >= tip {
                    info!(target: "pipeline", %tip, "Finished processing until tip.");
                    current_tip = None;
                    current_chunk_tip = last_block_processed;
                } else {
                    current_chunk_tip = (last_block_processed + self.chunk_size).min(tip);
                    continue;
                }
            }

            debug!(target: "pipeline", "Waiting for new command.");

            // Wait for the next command
            if self.command_rx.changed().await.is_err() {
                break;
            }
        }

        info!(target: "pipeline", "Pipeline finished.");

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
    /// The last block number that was successfully processed by all stages.
    ///
    /// # Errors
    ///
    /// Returns an error if any stage execution fails or if the pipeline fails to read the
    /// checkpoint.
    pub async fn run_to(&mut self, to: BlockNumber) -> PipelineResult<BlockNumber> {
        let last_stage_idx = self.stages.len() - 1;

        for (i, stage) in self.stages.iter_mut().enumerate() {
            let id = stage.id();

            // Get the checkpoint for the stage, otherwise default to block number 0
            let checkpoint = self.provider.checkpoint(id)?.unwrap_or_default();

            // Skip the stage if the checkpoint is greater than or equal to the target block number
            if checkpoint >= to {
                info!(target: "pipeline", %id, "Skipping stage.");

                if i == last_stage_idx {
                    return Ok(checkpoint);
                }

                continue;
            }

            info!(target: "pipeline", %id, from = %checkpoint, %to, "Executing stage.");

            // plus 1 because the checkpoint is inclusive
            let input = StageExecutionInput { from: checkpoint + 1, to };
            stage.execute(&input).await?;
            self.provider.set_checkpoint(id, to)?;

            info!(target: "pipeline", %id, from = %checkpoint, %to, "Stage execution completed.");
        }

        Ok(to)
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
