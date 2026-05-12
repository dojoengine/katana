use katana_primitives::block::BlockNumber;
use serde::{Deserialize, Serialize};

/// Unique identifier for a syncing pipeline stage.
pub type StageId = String;
/// Unique identifier for a database migration stage.
pub type MigrationStageId = String;

/// Pipeline stage checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(::arbitrary::Arbitrary))]
pub struct ExecutionCheckpoint {
    /// The block number that the stage has processed up to.
    pub block: BlockNumber,
}

/// Pipeline stage prune checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(::arbitrary::Arbitrary))]
pub struct PruningCheckpoint {
    /// The block number up to which the stage has been pruned (inclusive).
    pub block: BlockNumber,
}

/// Messaging service checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(::arbitrary::Arbitrary))]
pub struct MessagingCheckpoint {
    /// The settlement chain block number that was last successfully processed.
    pub block: u64,
    /// The transaction index within `block` up to which messages have been processed.
    pub tx_index: u64,
}

/// Checkpoint for a database migration task, storing the next key to process.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(::arbitrary::Arbitrary))]
pub struct MigrationCheckpoint {
    /// The most recently migrated key by the migration pipeline.
    pub last_key_migrated: u64,
}
