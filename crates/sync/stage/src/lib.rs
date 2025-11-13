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
        assert!(to >= from, "Invalid block range: `to` ({to}) must be >= `from` ({from})");
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

/// Pruning mode configuration that determines how much historical state to retain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruningMode {
    /// Keep all historical state since genesis (no pruning).
    Full,
    /// Keep only the last N blocks of historical state.
    HistoricalBlocks(u64),
    /// Keep only the latest state, pruning all historical data.
    LatestOnly,
}

impl Default for PruningMode {
    fn default() -> Self {
        Self::Full
    }
}

/// Input parameters for stage pruning.
#[derive(Debug, Clone)]
pub struct PruneInput {
    /// The current tip of the chain (highest synced block).
    tip: BlockNumber,
    /// The pruning mode to apply.
    mode: PruningMode,
}

impl PruneInput {
    /// Creates a new [`PruneInput`] with the given tip and pruning mode.
    pub fn new(tip: BlockNumber, mode: PruningMode) -> Self {
        Self { tip, mode }
    }

    /// Returns the current chain tip.
    #[inline]
    pub fn tip(&self) -> BlockNumber {
        self.tip
    }

    /// Returns the pruning mode.
    #[inline]
    pub fn mode(&self) -> PruningMode {
        self.mode
    }

    /// Calculates the block number before which all state should be pruned.
    ///
    /// Returns `None` if no pruning should occur (e.g., in `Full` mode).
    /// Returns `Some(block_number)` indicating that all state before this block can be pruned.
    pub fn prune_before(&self) -> Option<BlockNumber> {
        match self.mode {
            PruningMode::Full => None,
            PruningMode::LatestOnly => Some(self.tip.saturating_sub(1)),
            PruningMode::HistoricalBlocks(n) => {
                if self.tip > n {
                    Some(self.tip - n)
                } else {
                    None
                }
            }
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
}

#[cfg(test)]
mod tests {
    use crate::{PruneInput, PruningMode, StageExecutionInput};

    #[tokio::test]
    #[should_panic(expected = "Invalid block range")]
    async fn invalid_range_panics() {
        // When from > to, the range is invalid and should panic at construction time
        let _ = StageExecutionInput::new(100, 99);
    }

    #[test]
    fn prune_before_full_mode() {
        let input = PruneInput::new(1000, PruningMode::Full);
        assert_eq!(input.prune_before(), None);
    }

    #[test]
    fn prune_before_latest_only() {
        let input = PruneInput::new(1000, PruningMode::LatestOnly);
        assert_eq!(input.prune_before(), Some(999));

        // Edge case: tip at block 0
        let input = PruneInput::new(0, PruningMode::LatestOnly);
        assert_eq!(input.prune_before(), Some(0));
    }

    #[test]
    fn prune_before_historical_blocks() {
        // Keep last 100 blocks, tip at 1000
        let input = PruneInput::new(1000, PruningMode::HistoricalBlocks(100));
        assert_eq!(input.prune_before(), Some(900));

        // Keep last 100 blocks, tip at 50 (not enough blocks yet)
        let input = PruneInput::new(50, PruningMode::HistoricalBlocks(100));
        assert_eq!(input.prune_before(), None);

        // Keep last 100 blocks, tip at exactly 100
        let input = PruneInput::new(100, PruningMode::HistoricalBlocks(100));
        assert_eq!(input.prune_before(), Some(0));
    }
}
