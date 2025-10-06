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

/// Input parameters for stage execution.
///
/// # Invariant
///
/// The `to` field must always be greater than or equal to the `from` field (`to >= from`).
/// This invariant is enforced at construction time via the [`new`](Self::new) method and must be
/// maintained by all code paths that create this type.
#[derive(Debug, Clone)]
pub struct StageExecutionInput {
    pub from: BlockNumber,
    pub to: BlockNumber,
}

impl StageExecutionInput {
    /// Creates a new [`StageExecutionInput`] with the given range.
    ///
    /// # Panics
    ///
    /// Panics if `to < from`, as this violates the type's invariant.
    pub fn new(from: BlockNumber, to: BlockNumber) -> Self {
        assert!(to >= from, "Invalid block range: 'to' ({to}) must be >= 'from' ({from})");
        Self { from, to }
    }

    /// Creates a new [`StageExecutionInput`] without validating the range invariant.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `to >= from`. Violating this invariant may lead to
    /// unexpected behavior in [`Stage`] implementations.
    pub unsafe fn new_unchecked(from: BlockNumber, to: BlockNumber) -> Self {
        Self { from, to }
    }
}

impl Default for StageExecutionInput {
    fn default() -> Self {
        Self { from: 0, to: 0 }
    }
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
/// Stages are responsible for processing a range of blocks and updating the storage accordingly.
/// Each stage implementation can assume that the block range provided in [`StageExecutionInput`]
/// is valid (i.e., `input.to >= input.from`).
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

    /// Executes the stage for the given block range.
    ///
    /// # Contract
    ///
    /// Implementors can rely on the following guarantees:
    /// - The `input.to` field will always be greater than or equal to `input.from`
    /// - The block range `[input.from, input.to]` represents an inclusive range
    ///
    /// Implementors should process all blocks in the range `[input.from, input.to]` and
    /// update the storage accordingly. If an error occurs during processing, the stage
    /// should return an appropriate error variant from [`Error`].
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
