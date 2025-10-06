#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_provider::api::ProviderError;

pub mod blocks;
pub mod classes;
pub mod downloader;
mod sequencing;

pub use blocks::Blocks;
pub use classes::Classes;
pub use sequencing::Sequencing;

/// The result type of a stage execution. See [Stage::execute].
pub type StageResult = Result<(), Error>;

#[derive(Debug, Default, Clone)]
pub struct StageExecutionInput {
    pub from: BlockNumber,
    pub to: BlockNumber,
}

#[derive(Debug, Default)]
pub struct StageExecutionOutput {
    pub last_block_processed: BlockNumber,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Provider(#[from] ProviderError),

    /// Errors that could happen during the execution of the [`Blocks`](blocks::Blocks) stage.
    #[error(transparent)]
    Blocks(#[from] blocks::Error),

    /// Errors that could happen during the execution of the [`Classes`](classes::Classes) stage.
    #[error(transparent)]
    Classes(#[from] classes::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// A stage in the sync pipeline.
///
/// Stages are the building blocks of the sync pipeline. Each stage performs a specific task
/// in the synchronization process (e.g., downloading blocks, downloading classes, executing
/// transactions).
///
/// # Implementation Note
///
/// The [`execute`](Stage::execute) method returns a [`BoxFuture`] instead of `impl Future` to
/// maintain dyn-compatibility. This allows the pipeline to store different stage implementations
/// in a `Vec<Box<dyn Stage>>`, enabling dynamic composition of sync stages at runtime.
///
/// While this introduces a small heap allocation for the future, it's negligible compared to
/// the actual async work performed by stages (network I/O, database operations, etc.).
pub trait Stage: Send + Sync {
    /// Returns the id which uniquely identifies the stage.
    fn id(&self) -> &'static str;

    /// Executes the stage.
    ///
    /// # Arguments
    ///
    /// * `input` - The execution input containing the range of blocks to process
    ///
    /// # Returns
    ///
    /// A [`BoxFuture`] that resolves to a [`StageResult`] upon completion.
    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult>;
}
