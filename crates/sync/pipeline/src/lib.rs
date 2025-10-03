#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use core::future::IntoFuture;

use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::api::ProviderError;
use katana_stage::{Stage, StageExecutionInput};
use tokio::sync::watch;
use tracing::{error, info};

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

/// A handle for controlling a running pipeline.
///
/// This handle allows external code to update the target tip block that the pipeline
/// should sync to.
#[derive(Debug, Clone)]
pub struct PipelineHandle {
    tx: watch::Sender<Option<BlockNumber>>,
}

impl PipelineHandle {
    /// Sets the target tip block for the pipeline to sync to.
    ///
    /// The pipeline will process all blocks up to and including this block number.
    /// This method will wake up the pipeline if it's currently waiting for a new tip.
    ///
    /// # Panics
    ///
    /// Panics if the pipeline has been dropped and the channel is closed.
    pub fn set_tip(&self, tip: BlockNumber) {
        info!(target: "pipeline", %tip, "Setting new tip");
        self.tx.send(Some(tip)).expect("channel closed");
    }
}

/// Syncing pipeline.
///
/// The pipeline drives the execution of stages, running each stage to completion in the order they
/// were added.
pub struct Pipeline<P> {
    chunk_size: u64,
    provider: P,
    stages: Vec<Box<dyn Stage>>,
    tip_watcher: (watch::Receiver<Option<BlockNumber>>, watch::Sender<Option<BlockNumber>>),
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
        let pipeline = Self { stages: Vec::new(), tip_watcher: (rx, tx), provider, chunk_size };
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
    /// The handle can be used to set the target tip block for the pipeline to sync to.
    pub fn handle(&self) -> PipelineHandle {
        PipelineHandle { tx: self.tip_watcher.1.clone() }
    }
}

impl<P: StageCheckpointProvider> Pipeline<P> {
    /// Runs the pipeline continuously until signaled to stop.
    ///
    /// The pipeline processes blocks in chunks up to the current tip, then waits for the tip
    /// to be updated via the [`PipelineHandle::set_tip`].
    ///
    /// # Errors
    ///
    /// Returns an error if any stage execution fails or if there's a provider error.
    pub async fn run(&mut self) -> PipelineResult<()> {
        let mut current_chunk_tip = self.chunk_size;

        loop {
            let tip = *self.tip_watcher.0.borrow_and_update();

            while let Some(tip) = tip {
                let to = current_chunk_tip.min(tip);
                let last_block_processed = self.run_to(to).await?;

                if last_block_processed >= tip {
                    info!(target: "pipeline", %tip, "Finished processing until tip.");
                    break;
                } else {
                    current_chunk_tip = (last_block_processed + self.chunk_size).min(tip);
                }
            }

            info!(target: "pipeline", "Waiting for new tip.");

            // If we reach here, that means we have run the pipeline up until the `tip`.
            // So, wait until the tip has changed.
            if self.tip_watcher.0.changed().await.is_err() {
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
            .field("tip", &self.tip_watcher)
            .field("provider", &self.provider)
            .field("chunk_size", &self.chunk_size)
            .field("stages", &self.stages.iter().map(|s| s.id()).collect::<Vec<_>>())
            .finish()
    }
}
