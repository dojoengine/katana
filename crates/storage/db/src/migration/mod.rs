mod receipt_envelopes;
mod state_updates;

use katana_primitives::block::BlockNumber;

pub(crate) use self::receipt_envelopes::ReceiptEnvelopeStage;
pub(crate) use self::state_updates::StateUpdatesStage;
use crate::error::DatabaseError;
use crate::version::{self, Version, LATEST_DB_VERSION};
use crate::Db;

/// Errors that can occur during database migration.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// A database operation failed during migration.
    #[error("database error: {0}")]
    Database(#[from] DatabaseError),

    /// Reconstructing state updates for a specific block failed.
    #[error("failed to reconstruct state update for block {block}")]
    FailedToReconstructStateUpdate {
        /// The block number for which state update reconstruction failed.
        block: BlockNumber,

        #[source]
        source: DatabaseError,
    },

    /// Failed to update the database version file after migration.
    #[error("failed to update database version: {0}")]
    VersionUpdate(#[from] version::DatabaseVersionError),
}

const BATCH_SIZE: u64 = 1000;

/// A single, self-contained migration step executed by [`Migration`].
///
/// The pipeline calls [`is_needed`](MigrationStage::is_needed) for every registered stage and,
/// for those that return `true`, invokes [`execute`](MigrationStage::execute) in registration
/// order. After **all** stages complete the pipeline bumps the on-disk version file to
/// [`LATEST_DB_VERSION`].
///
/// # Implementing a new stage
///
/// 1. Create a unit struct (e.g. `pub(crate) struct MyStage;`).
/// 2. Implement the three required methods below.
/// 3. Register the stage in [`Migration::new`] via `m.add_stage(Box::new(MyStage))`.
///
/// ## Checkpointing
///
/// Long-running stages should persist progress to the
/// [`MigrationCheckpoints`](crate::tables::MigrationCheckpoints) table so that a crash-interrupted
/// migration can resume from the last committed batch instead of restarting from scratch.
/// On the final batch, the checkpoint entry **must** be deleted so it does not persist after a
/// successful migration.
///
/// See [`StateUpdatesStage`] and [`ReceiptEnvelopeStage`] for reference implementations.
pub trait MigrationStage {
    /// A unique, human-readable identifier for this stage.
    ///
    /// Used as the key in [`MigrationCheckpoints`](crate::tables::MigrationCheckpoints) and in
    /// progress-bar labels printed to stderr. Must be a `&'static str` constant — by convention,
    /// use the form `"migration/<short-name>"` (e.g. `"migration/state-updates"`).
    fn id(&self) -> &'static str;

    /// The minimum database version that **already contains** this stage's data.
    ///
    /// The stage is skipped when `db.version() >= threshold_version()`. Return the version in
    /// which the schema change (or data format change) that this stage addresses was first
    /// introduced. For example, if the `BlockStateUpdates` table was added in version 9, return
    /// `Version::new(9)` so that databases already at version 9 or later are not migrated.
    fn threshold_version(&self) -> Version;

    /// Returns `true` if this stage needs to run for `db`.
    ///
    /// The default implementation compares `db.version()` against
    /// [`threshold_version`](MigrationStage::threshold_version). Override only if additional
    /// conditions beyond a simple version check are required.
    fn is_needed(&self, db: &Db) -> bool {
        db.version() < self.threshold_version()
    }

    /// Performs the actual migration work.
    ///
    /// Implementations should:
    ///
    /// - Process rows in batches of [`BATCH_SIZE`] to bound memory usage and allow
    ///   crash-resumable checkpointing.
    /// - Write a [`MigrationCheckpoint`](crate::models::stage::MigrationCheckpoint) at the end of
    ///   every non-final batch (within the same write transaction) and delete it on the final
    ///   batch.
    /// - Display progress via an [`indicatif::ProgressBar`] written to stderr.
    /// - Return `Ok(())` when there is nothing to migrate (e.g. empty table, already up-to-date).
    fn execute(&self, db: &Db) -> Result<(), MigrationError>;
}

/// Runs all applicable database migrations based on the on-disk version at open time.
///
/// Each migration path has its own version threshold. Only migrations whose version
/// is above the database's opened version will be executed.
pub struct Migration<'a> {
    db: &'a Db,
    stages: Vec<Box<dyn MigrationStage>>,
}

impl std::fmt::Debug for Migration<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("stages", &self.stages.iter().map(|s| s.id()).collect::<Vec<_>>())
            .finish()
    }
}

impl<'a> Migration<'a> {
    pub fn new(db: &'a Db) -> Self {
        let mut m = Self { db, stages: Vec::new() };
        m.add_stage(Box::new(StateUpdatesStage));
        m.add_stage(Box::new(ReceiptEnvelopeStage));
        m
    }

    pub fn add_stage(&mut self, stage: Box<dyn MigrationStage>) {
        self.stages.push(stage);
    }

    /// Returns `true` if any migration stage needs to be run.
    pub fn is_needed(&self) -> bool {
        self.stages.iter().any(|s| s.is_needed(self.db))
    }

    /// Runs the migration process.
    pub fn run(&self) -> Result<(), MigrationError> {
        eprintln!("[Migrating] Starting migration");

        for stage in &self.stages {
            if stage.is_needed(self.db) {
                stage.execute(self.db)?;
            }
        }

        // Update the on-disk version file to the latest version.
        version::write_db_version_file(self.db.path(), LATEST_DB_VERSION)?;

        eprintln!(
            "[Migrating] Migration complete (version updated to \x1b[1m{LATEST_DB_VERSION}\x1b[0m)"
        );

        Ok(())
    }
}
