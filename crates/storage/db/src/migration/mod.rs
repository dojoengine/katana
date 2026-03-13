mod receipt_envelopes;
mod state_updates;

use std::ops::RangeInclusive;

use indicatif::{ProgressBar, ProgressStyle};
use katana_primitives::block::BlockNumber;

pub(crate) use self::receipt_envelopes::ReceiptEnvelopeStage;
pub(crate) use self::state_updates::StateUpdatesStage;
use crate::abstraction::{Database, DbTx, DbTxMut};
use crate::error::DatabaseError;
use crate::mdbx::tx::TxRW;
use crate::models::stage::MigrationCheckpoint;
use crate::version::{self, Version, LATEST_DB_VERSION};
use crate::{tables, Db};

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

/// A single, self-contained migration step executed by [`Migration`].
///
/// The pipeline compares each stage's [`threshold_version`](MigrationStage::threshold_version)
/// against the database's on-disk version. Stages whose threshold is above the current version
/// are executed in registration order. After **all** stages complete, the pipeline bumps the
/// on-disk version file to [`LATEST_DB_VERSION`].
///
/// ## How the pipeline drives a stage
///
/// 1. Calls [`range`](MigrationStage::range) to obtain the full key range that needs migration. If
///    `None`, the stage is skipped.
///
/// 2. Looks up the checkpoint for [`id`](MigrationStage::id) to adjust the start of the range.
///
/// 3. Partitions the remaining range into batches of [`BATCH_SIZE`] and, for each batch: opens a
///    write transaction, calls [`execute`](MigrationStage::execute) with the batch range, writes
///    the checkpoint (or deletes it on the final batch), and commits — all in one atomic
///    transaction.
///
/// The [`Migration`] pipeline handles all the checkpointing. Stage only process data within a
/// pipeline-provided write transaction and range.
///
/// See [`StateUpdatesStage`] and [`ReceiptEnvelopeStage`] for reference implementations.
pub trait MigrationStage {
    /// A unique, human-readable identifier for this stage.
    fn id(&self) -> &'static str;

    /// The minimum database version that **already contains** this stage's data.
    ///
    /// The stage is skipped when `db.version() >= threshold_version()`. Return the version in
    /// which the schema change (or data format change) that this stage addresses was first
    /// introduced.
    ///
    /// For example, if the `BlockStateUpdates` table was added in version 9, return
    /// `Version::new(9)` so that databases already at version 9 or later are not migrated.
    fn threshold_version(&self) -> Version;

    /// Returns the inclusive key range that needs migration, or `None` if there is nothing to
    /// migrate (e.g. empty table, no blocks).
    ///
    /// The keys are stage-specific — block numbers, transaction numbers, etc. The pipeline
    /// treats them as opaque `u64` values and only uses them for batching and checkpoint
    /// arithmetic.
    fn range(&self, db: &Db) -> Result<Option<RangeInclusive<u64>>, MigrationError>;

    /// Process all items in the given key `range` within the provided write transaction.
    ///
    /// The `range` is an inclusive sub-range of the full range returned by
    /// [`range()`](MigrationStage::range), partitioned by the pipeline according to
    /// its configured batch size.
    ///
    /// The keys carry stage-specific semantics — for example, block numbers in or transaction
    /// sequence numbers. The pipeline is unaware of these semantics; it only performs
    /// arithmetic on the `u64` boundaries for batching and checkpoint storage. Implementations
    /// MUST process **every** key in the range.
    fn execute(&self, tx: &TxRW, range: RangeInclusive<u64>) -> Result<(), MigrationError>;
}

/// Number of key-space units per batch. The pipeline partitions the stage's
/// [`range`](MigrationStage::range) into chunks of this size and calls
/// [`execute`](MigrationStage::execute) once per chunk.
const BATCH_SIZE: u64 = 1000;

/// Runs all applicable database migrations based on the on-disk version at open time.
///
/// Each migration path has its own version threshold. Only migrations whose version
/// is above the database's opened version will be executed.
pub struct Migration<'a> {
    db: &'a Db,
    stages: Vec<Box<dyn MigrationStage>>,
}

impl<'a> Migration<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db, stages: Vec::new() }
    }

    /// Creates a new [`Migration`] with all the steps for migrating to database version 9.
    pub fn new_v9(db: &'a Db) -> Self {
        let mut m = Self::new(db);
        m.add_migration(Box::new(StateUpdatesStage));
        m.add_migration(Box::new(ReceiptEnvelopeStage));
        m
    }

    /// Adds a migration step.
    pub fn add_migration(&mut self, stage: Box<dyn MigrationStage>) {
        self.stages.push(stage);
    }

    /// Returns `true` if any migration stage needs to be run.
    pub fn is_needed(&self) -> bool {
        self.stages.iter().any(|s| self.stage_needed(s.as_ref()))
    }

    /// Runs the migration process.
    pub fn run(&self) -> Result<(), MigrationError> {
        eprintln!("[Migrating] Starting migration");

        for stage in &self.stages {
            if self.stage_needed(stage.as_ref()) {
                self.run_stage(stage.as_ref())?;
            }
        }

        // Update the on-disk version file to the latest version.
        version::write_db_version_file(self.db.path(), LATEST_DB_VERSION)?;

        eprintln!(
            "[Migrating] Migration complete (version updated to \x1b[1m{LATEST_DB_VERSION}\x1b[0m)"
        );

        Ok(())
    }

    /// Returns `true` if the database version is below any stage's threshold.
    fn stage_needed(&self, stage: &dyn MigrationStage) -> bool {
        self.db.version() < stage.threshold_version()
    }

    /// Drives a single stage through its batch loop with checkpoint management.
    fn run_stage(&self, stage: &dyn MigrationStage) -> Result<(), MigrationError> {
        let db = self.db;
        let id = stage.id();

        let full_range = match stage.range(db)? {
            Some(r) => r,
            None => return Ok(()),
        };

        let range_end = *full_range.end();

        // Resume from the last checkpoint if one exists.
        let checkpoint = db.view(|tx| tx.get::<tables::MigrationCheckpoints>(id.to_string()))?;
        let range_start = checkpoint.map(|cp| cp.next_key).unwrap_or(*full_range.start());

        if range_start > range_end {
            // Already complete — clean up the stale checkpoint.
            db.update(|tx| {
                tx.delete::<tables::MigrationCheckpoints>(id.to_string(), None)?;
                Ok(())
            })?;
            return Ok(());
        }

        let remaining = range_end - range_start + 1;

        let pb = ProgressBar::new(remaining);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&format!(
                    "[Migrating] \x1b[1;33m{id}\x1b[0m {{bar:40.cyan/blue}} {{pos}}/{{len}} \
                     [{{elapsed_precise}}] {{per_sec}}"
                ))
                .expect("valid format"),
        );

        // Partition the remaining range into batches.
        let mut batch_start = range_start;
        while batch_start <= range_end {
            let batch_end = std::cmp::min(batch_start + BATCH_SIZE - 1, range_end);
            let is_last = batch_end == range_end;

            let tx = db.tx_mut()?;
            stage.execute(&tx, batch_start..=batch_end)?;

            if is_last {
                // Final batch — remove the checkpoint.
                tx.delete::<tables::MigrationCheckpoints>(id.to_string(), None)?;
            } else {
                // Intermediate batch — persist checkpoint atomically with data.
                tx.put::<tables::MigrationCheckpoints>(
                    id.to_string(),
                    MigrationCheckpoint { next_key: batch_end + 1 },
                )?;
            }

            tx.commit()?;
            pb.set_position(batch_end - range_start + 1);

            batch_start = batch_end + 1;
        }

        pb.finish();

        Ok(())
    }
}

impl std::fmt::Debug for Migration<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("db", &self.db)
            .field("stages", &self.stages.iter().map(|s| s.id()).collect::<Vec<_>>())
            .finish()
    }
}
