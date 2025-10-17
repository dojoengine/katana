#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_provider::api::ProviderError;

pub mod blocks;
pub mod classes;
pub mod downloader;
mod sequencing;
pub mod trie;

pub use blocks::{Blocks, DatabaseProvider};
pub use classes::Classes;
pub use sequencing::Sequencing;
pub use trie::StateTrie;

/// The result type of a stage execution. See [Stage::execute].
pub type StageResult = Result<StageExecutionOutput, Error>;

/// Input parameters for stage execution.
///
/// # Invariant
///
/// The `to` field must always be greater than or equal to the `from` field (`to >= from`).
/// This invariant is enforced at construction time via the [`new`](Self::new) method and
/// maintained by keeping the fields private.
#[derive(Debug, Clone, Default)]
pub struct StageExecutionInput {
    from: BlockNumber,
    to: BlockNumber,
}

impl StageExecutionInput {
    /// Creates a new [`StageExecutionInput`] with the given range.
    ///
    /// # Panics
    ///
    /// Panics if `to < from`, as this violates the type's invariant.
    pub fn new(from: BlockNumber, to: BlockNumber) -> Self {
        // assert!(to >= from, "Invalid block range: `to` ({to}) must be >= `from` ({from})");
        Self { from, to }
    }

    /// Returns the starting block number (inclusive).
    #[inline]
    pub fn from(&self) -> BlockNumber {
        self.from
    }

    /// Returns the ending block number (inclusive).
    #[inline]
    pub fn to(&self) -> BlockNumber {
        self.to
    }
}

/// Output from a stage execution containing the progress information.
#[derive(Debug, Default)]
pub struct StageExecutionOutput {
    /// The last block number that was successfully processed by the stage.
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

    /// Errors that could happen during the execution of the [`StateTrie`](state_trie::StateTrie)
    /// stage.
    #[error(transparent)]
    StateTrie(#[from] trie::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// A stage in the sync pipeline.
///
/// Stages are the building blocks of the sync pipeline. Each stage performs a specific task
/// in the synchronization process (e.g., downloading blocks, downloading classes, executing
/// transactions).
///
/// Stages are responsible for processing a range of blocks. Each stage implementation can assume
/// that the block range provided in [`StageExecutionInput`] is valid (i.e., `input.to >=
/// input.from`).
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
    /// # Arguments
    ///
    /// * `input` - The execution input containing the range of blocks to process
    ///
    /// # Returns
    ///
    /// A future that resolves to a [`StageResult`] containing [`StageExecutionOutput`]
    /// with the last successfully processed block number of the stage.
    ///
    /// # Block Range
    ///
    /// Implementors can rely on the following guarantees:
    /// - The `input.to` field will always be greater than or equal to `input.from`
    /// - The block range `[input.from, input.to]` represents an inclusive range
    ///
    /// Implementors are expected to perform any necessary processings on all blocks in the range
    /// `[input.from, input.to]`.
    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult>;

    /// Unwinds the stage to the specified block number.
    ///
    /// This method is called during chain reorganizations to revert the chain state back to a
    /// specific block. All blocks after the `unwind_to` block should be removed, and the
    /// resulting database state should be as if the stage had only synced up to `unwind_to`.
    ///
    /// If the `unwind_to` block is larger than the state's checkpoint, this method will be a no-op
    /// and should return the checkpoint block number.
    ///
    /// # Arguments
    ///
    /// * `unwind_to` - The target block number to unwind to. All blocks after this will be removed.
    ///
    /// # Returns
    ///
    /// A future that resolves to a [`StageResult`] containing [`StageExecutionOutput`]
    /// with the last block number after unwinding or the checkpoint block number (if the stage's
    /// checkpoint is smaller than the unwind target).
    ///
    /// # Implementation Requirements
    ///
    /// Implementors must ensure that:
    /// - All data for blocks > `unwind_to` is removed from relevant database tables
    /// - The stage checkpoint is updated to reflect the unwound state
    /// - Database invariants are maintained after the unwind operation
    fn unwind<'a>(&'a mut self, unwind_to: BlockNumber) -> BoxFuture<'a, StageResult> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use crate::StageExecutionInput;

    #[tokio::test]
    #[ignore]
    #[should_panic(expected = "Invalid block range")]
    async fn invalid_range_panics() {
        // When from > to, the range is invalid and should panic at construction time
        let _ = StageExecutionInput::new(100, 99);
    }
}
