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

        // Compute the longest stage ID among applicable stages so progress bars align.
        let label_width = self
            .stages
            .iter()
            .filter(|s| self.stage_needed(s.as_ref()))
            .map(|s| s.id().len())
            .max()
            .unwrap_or(0);

        for stage in &self.stages {
            if self.stage_needed(stage.as_ref()) {
                self.run_stage(stage.as_ref(), label_width)?;
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
    fn run_stage(
        &self,
        stage: &dyn MigrationStage,
        label_width: usize,
    ) -> Result<(), MigrationError> {
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

        // Pad the label so all progress bars align vertically.
        let padded_id = format!("{id:<label_width$}");

        let pb = ProgressBar::new(remaining);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&format!(
                    "[Migrating] \x1b[1;33m{padded_id}\x1b[0m {{bar:40.cyan/blue}} \
                     {{pos}}/{{len}} [{{elapsed_precise}}] {{per_sec}}"
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

#[cfg(test)]
mod tests {
    use katana_primitives::state::StateUpdates;

    use super::*;
    use crate::models::stage::MigrationCheckpoint;
    use crate::version::create_db_version_file;

    /// Minimal stage for testing pipeline logic in isolation.
    ///
    /// Writes `StateUpdates::default()` for each key in a fixed `0..=count-1` range to
    /// `BlockStateUpdates`, so pipeline tests can verify batching, checkpointing, and
    /// version management without depending on real migration stages.
    struct MarkerStage {
        id: &'static str,
        count: u64,
    }

    impl MarkerStage {
        fn new(count: u64) -> Self {
            Self { id: "test/marker", count }
        }

        fn with_id(id: &'static str, count: u64) -> Self {
            Self { id, count }
        }
    }

    impl MigrationStage for MarkerStage {
        fn id(&self) -> &'static str {
            self.id
        }

        fn threshold_version(&self) -> Version {
            Version::new(9)
        }

        fn range(&self, _db: &Db) -> Result<Option<RangeInclusive<u64>>, MigrationError> {
            if self.count == 0 {
                Ok(None)
            } else {
                Ok(Some(0..=self.count - 1))
            }
        }

        fn execute(&self, tx: &TxRW, range: RangeInclusive<u64>) -> Result<(), MigrationError> {
            for key in range {
                tx.put::<tables::BlockStateUpdates>(key, StateUpdates::default())?;
            }
            Ok(())
        }
    }

    fn old_version_db() -> (Db, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        create_db_version_file(dir.path(), Version::new(8)).unwrap();
        let db = Db::open_no_sync(dir.path()).unwrap();
        (db, dir)
    }

    fn pipeline_with_marker(db: &Db, count: u64) -> Migration<'_> {
        let mut m = Migration::new(db);
        m.add_migration(Box::new(MarkerStage::new(count)));
        m
    }

    #[test]
    fn skips_stage_at_current_version() {
        let db = Db::in_memory().unwrap();
        assert!(!pipeline_with_marker(&db, 10).is_needed());
    }

    #[test]
    fn runs_stage_below_threshold() {
        let (db, _dir) = old_version_db();
        assert!(pipeline_with_marker(&db, 10).is_needed());
    }

    #[test]
    fn updates_version_file() {
        let (db, dir) = old_version_db();
        pipeline_with_marker(&db, 5).run().unwrap();
        let v = version::get_db_version(dir.path()).unwrap();
        assert_eq!(v, LATEST_DB_VERSION);
    }

    #[test]
    fn not_needed_after_run() {
        let (db, dir) = old_version_db();
        pipeline_with_marker(&db, 5).run().unwrap();
        drop(db);
        let db2 = Db::open_no_sync(dir.path()).unwrap();
        assert!(!pipeline_with_marker(&db2, 5).is_needed());
    }

    #[test]
    fn empty_range_is_noop() {
        let (db, _dir) = old_version_db();
        pipeline_with_marker(&db, 0).run().unwrap();
        let count = db.view(|tx| tx.entries::<tables::BlockStateUpdates>()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn resumes_from_checkpoint() {
        let (db, _dir) = old_version_db();
        let total = 10u64;
        let checkpoint_at = 5u64;

        // Pre-populate entries 0..5 and set checkpoint.
        {
            let tx = db.tx_mut().unwrap();
            for i in 0..checkpoint_at {
                tx.put::<tables::BlockStateUpdates>(i, StateUpdates::default()).unwrap();
            }
            tx.put::<tables::MigrationCheckpoints>(
                "test/marker".to_string(),
                MigrationCheckpoint { next_key: checkpoint_at },
            )
            .unwrap();
            tx.commit().unwrap();
        }

        pipeline_with_marker(&db, total).run().unwrap();

        let tx = db.tx().unwrap();
        let count = tx.entries::<tables::BlockStateUpdates>().unwrap();
        let cp = tx.get::<tables::MigrationCheckpoints>("test/marker".to_string()).unwrap();
        tx.commit().unwrap();

        assert_eq!(count, total as usize);
        assert!(cp.is_none(), "checkpoint should be removed after migration completes");
    }

    #[test]
    fn cleans_up_stale_checkpoint() {
        let (db, _dir) = old_version_db();

        // Stage has 5 items, but checkpoint points past the end.
        {
            let tx = db.tx_mut().unwrap();
            for i in 0..5u64 {
                tx.put::<tables::BlockStateUpdates>(i, StateUpdates::default()).unwrap();
            }
            tx.put::<tables::MigrationCheckpoints>(
                "test/marker".to_string(),
                MigrationCheckpoint { next_key: 100 },
            )
            .unwrap();
            tx.commit().unwrap();
        }

        pipeline_with_marker(&db, 5).run().unwrap();

        let cp = db
            .view(|tx| tx.get::<tables::MigrationCheckpoints>("test/marker".to_string()))
            .unwrap();
        assert!(cp.is_none(), "stale checkpoint should be cleaned up");
    }

    #[test]
    fn batches_large_range() {
        let (db, _dir) = old_version_db();
        let total = 1500u64; // exceeds BATCH_SIZE (1000)

        pipeline_with_marker(&db, total).run().unwrap();

        let count = db.view(|tx| tx.entries::<tables::BlockStateUpdates>()).unwrap();
        assert_eq!(count, total as usize);
    }

    #[test]
    fn checkpoint_resumes_across_batches() {
        let (db, _dir) = old_version_db();
        let total = 1500u64;
        let checkpoint_at = 1000u64;

        // Pre-populate first batch and set checkpoint at batch boundary.
        {
            let tx = db.tx_mut().unwrap();
            for i in 0..checkpoint_at {
                tx.put::<tables::BlockStateUpdates>(i, StateUpdates::default()).unwrap();
            }
            tx.put::<tables::MigrationCheckpoints>(
                "test/marker".to_string(),
                MigrationCheckpoint { next_key: checkpoint_at },
            )
            .unwrap();
            tx.commit().unwrap();
        }

        pipeline_with_marker(&db, total).run().unwrap();

        let tx = db.tx().unwrap();
        let count = tx.entries::<tables::BlockStateUpdates>().unwrap();
        let cp = tx.get::<tables::MigrationCheckpoints>("test/marker".to_string()).unwrap();
        tx.commit().unwrap();

        assert_eq!(count, total as usize);
        assert!(cp.is_none(), "checkpoint should be removed after migration completes");
    }

    #[test]
    fn no_checkpoint_after_full_migration() {
        let (db, _dir) = old_version_db();
        pipeline_with_marker(&db, 5).run().unwrap();

        let total = db.view(|tx| tx.entries::<tables::MigrationCheckpoints>()).unwrap();
        assert_eq!(total, 0, "no migration checkpoints should remain");
    }

    #[test]
    fn independent_stage_checkpoints() {
        let (db, _dir) = old_version_db();

        // Pre-populate partial data and checkpoint for stage A only.
        {
            let tx = db.tx_mut().unwrap();
            tx.put::<tables::BlockStateUpdates>(0, StateUpdates::default()).unwrap();
            tx.put::<tables::MigrationCheckpoints>(
                "test/stage-a".to_string(),
                MigrationCheckpoint { next_key: 1 },
            )
            .unwrap();
            tx.commit().unwrap();
        }

        let mut m = Migration::new(&db);
        m.add_migration(Box::new(MarkerStage::with_id("test/stage-a", 3)));
        m.add_migration(Box::new(MarkerStage::with_id("test/stage-b", 3)));
        m.run().unwrap();

        let tx = db.tx().unwrap();
        let count = tx.entries::<tables::BlockStateUpdates>().unwrap();
        let cp_a = tx.get::<tables::MigrationCheckpoints>("test/stage-a".to_string()).unwrap();
        let cp_b = tx.get::<tables::MigrationCheckpoints>("test/stage-b".to_string()).unwrap();
        let total_checkpoints = tx.entries::<tables::MigrationCheckpoints>().unwrap();
        tx.commit().unwrap();

        assert_eq!(count, 3);
        assert!(cp_a.is_none(), "stage A checkpoint should be removed");
        assert!(cp_b.is_none(), "stage B checkpoint should be removed");
        assert_eq!(total_checkpoints, 0, "no migration checkpoints should remain");
    }
}
