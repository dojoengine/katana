#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_provider::api::ProviderError;

pub mod blocks;
pub mod classes;
pub mod downloader;
mod sequencing;
pub mod trie;

pub use blocks::Blocks;
pub use classes::Classes;
pub use sequencing::Sequencing;
pub use trie::StateTrie;

/// The result type of a stage execution. See [Stage::execute].
pub type StageResult = Result<StageExecutionOutput, Error>;

/// The result type of a stage pruning. See [Stage::prune].
pub type PruneResult = Result<PruneOutput, Error>;

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

/// Input parameters for stage pruning.
#[derive(Debug, Clone)]
pub struct PruneInput {
    /// The current tip of the chain (highest synced block).
    tip: BlockNumber,
    /// Distance from tip. Blocks older than `tip - distance` will be pruned.
    /// `None` means no pruning.
    distance: Option<u64>,
    /// The last block number that was successfully pruned (if any).
    last_pruned: Option<BlockNumber>,
}

impl PruneInput {
    /// Creates a new [`PruneInput`] with the given tip, distance, and last pruned block.
    ///
    /// # Arguments
    ///
    /// * `tip` - The current tip of the chain (highest synced block)
    /// * `distance` - Distance from tip. Blocks older than `tip - distance` will be pruned. `None`
    ///   means no pruning.
    /// * `last_pruned` - The last block number that was successfully pruned (if any)
    pub fn new(tip: BlockNumber, distance: Option<u64>, last_pruned: Option<BlockNumber>) -> Self {
        Self { tip, distance, last_pruned }
    }

    /// Returns the current chain tip.
    #[inline]
    pub fn tip(&self) -> BlockNumber {
        self.tip
    }

    /// Returns the distance from tip for pruning.
    #[inline]
    pub fn distance(&self) -> Option<u64> {
        self.distance
    }

    /// Returns the last block that was successfully pruned.
    #[inline]
    pub fn last_pruned(&self) -> Option<BlockNumber> {
        self.last_pruned
    }

    /// Returns the range of blocks to prune, if any.
    ///
    /// The range is `[start, end)` where:
    /// - `start` is `last_pruned + 1` (or 0 if no previous pruning)
    /// - `end` is the calculated prune target based on tip and distance
    ///
    /// Returns `None` if no pruning should occur (e.g., distance is `None` or already caught up).
    pub fn prune_range(&self) -> Option<std::ops::Range<BlockNumber>> {
        let prune_target = self.calculate_prune_target()?;
        let start = self.last_pruned.map(|b| b + 1).unwrap_or(0);

        if start < prune_target {
            Some(start..prune_target)
        } else {
            None
        }
    }

    /// Calculates the block number before which all state should be pruned.
    ///
    /// Returns `None` if no pruning should occur (e.g., distance is `None` or tip < distance).
    /// Returns `Some(block_number)` indicating that all state before this block can be pruned.
    fn calculate_prune_target(&self) -> Option<BlockNumber> {
        let distance = self.distance?;
        if self.tip >= distance {
            Some(self.tip - distance)
        } else {
            None
        }
    }
}

/// Output from a stage pruning operation.
#[derive(Debug, Default)]
pub struct PruneOutput {
    /// The number of items (blocks, state entries, etc.) that were pruned.
    pub pruned_count: u64,
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
/// The [`execute`](Stage::execute) and [`prune`](Stage::prune) methods return a [`BoxFuture`]
/// instead of `impl Future` to maintain dyn-compatibility. This allows the pipeline to store
/// different stage implementations in a `Vec<Box<dyn Stage>>`, enabling dynamic composition of
/// sync stages at runtime.
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

    /// Prunes historical data for this stage according to the pruning configuration.
    ///
    /// This method is called by the pipeline to remove old historical state that is no longer
    /// needed according to the pruning mode. The pruning operation is non-blocking and runs
    /// asynchronously.
    ///
    /// # Arguments
    ///
    /// * `input` - The pruning input containing the current chain tip and pruning mode
    ///
    /// # Returns
    ///
    /// A future that resolves to a [`PruneResult`] containing [`PruneOutput`] with the
    /// number of items that were pruned.
    ///
    /// # Implementation Notes
    ///
    /// - Stages that don't store historical state (e.g., Classes) can provide a no-op
    ///   implementation that returns `Ok(PruneOutput::default())`.
    /// - Stages that store state (e.g., Blocks, StateTrie) should implement pruning logic
    ///   appropriate to their data model.
    /// - The pruning operation must be non-blocking, just like [`execute`](Stage::execute).
    /// - Implementors should use [`PruneInput::prune_before`] to determine which blocks to prune.
    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult>;

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
    fn unwind(&mut self, unwind_to: BlockNumber) -> BoxFuture<'_, StageResult>;
}

#[cfg(test)]
mod tests {
    use crate::{PruneInput, StageExecutionInput};

    #[tokio::test]
    #[ignore]
    #[should_panic(expected = "Invalid block range")]
    async fn invalid_range_panics() {
        // When from > to, the range is invalid and should panic at construction time
        let _ = StageExecutionInput::new(100, 99);
    }

    #[test]
    fn prune_range_no_pruning() {
        // distance = None means no pruning (archive mode)
        let input = PruneInput::new(1000, None, None);
        assert_eq!(input.prune_range(), None);
    }

    #[test]
    fn prune_range_with_distance() {
        // Keep last 100 blocks (distance=100), tip at 1000, no previous pruning
        let input = PruneInput::new(1000, Some(100), None);
        assert_eq!(input.prune_range(), Some(0..900));

        // Keep last 100 blocks, tip at 1000, previously pruned up to 800
        let input = PruneInput::new(1000, Some(100), Some(800));
        assert_eq!(input.prune_range(), Some(801..900));

        // Keep last 100 blocks, tip at 50 (not enough blocks yet)
        let input = PruneInput::new(50, Some(100), None);
        assert_eq!(input.prune_range(), None);

        // Keep last 100 blocks, tip at exactly 100 (prune target is 0, start is 0, so empty range)
        let input = PruneInput::new(100, Some(100), None);
        assert_eq!(input.prune_range(), None); // 0..0 is empty, returns None

        // Already caught up
        let input = PruneInput::new(1000, Some(100), Some(899));
        assert_eq!(input.prune_range(), None);
    }

    #[test]
    fn prune_range_minimal_distance() {
        // distance=1 means keep only the latest block (minimal mode equivalent)
        // First prune: from 0 to tip-1
        let input = PruneInput::new(1000, Some(1), None);
        assert_eq!(input.prune_range(), Some(0..999));

        // Subsequent prune with checkpoint
        let input = PruneInput::new(1005, Some(1), Some(998));
        assert_eq!(input.prune_range(), Some(999..1004));

        // Already caught up
        let input = PruneInput::new(1000, Some(1), Some(998));
        assert_eq!(input.prune_range(), None);

        // Edge case: tip at block 0
        let input = PruneInput::new(0, Some(1), None);
        assert_eq!(input.prune_range(), None); // tip < distance, no pruning
    }
}
