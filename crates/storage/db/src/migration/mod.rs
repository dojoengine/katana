mod receipt_envelopes;
mod state_updates;

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

const BATCH_SIZE: u64 = 1000;

/// A single, self-contained migration step executed by [`Migration`].
///
/// The pipeline compares each stage's [`threshold_version`](MigrationStage::threshold_version)
/// against the database's on-disk version. Stages whose threshold is above the current version
/// are executed in registration order. After **all** stages complete, the pipeline bumps the
/// on-disk version file to [`LATEST_DB_VERSION`].
///
/// ## How the pipeline drives a stage
///
/// 1. Calls [`total_items`](MigrationStage::total_items) to determine work size and set up a
///    progress bar. If 0, the stage is skipped.
/// 2. Looks up the checkpoint for [`id`](MigrationStage::id) to find the resume position.
/// 3. Enters a batch loop: opens a write transaction, calls
///    [`process_batch`](MigrationStage::process_batch), writes the checkpoint (or deletes it on the
///    final batch), and commits — all in one atomic transaction.
///
/// Stages never read or write checkpoints themselves. They only process data within a
/// pipeline-provided write transaction.
///
/// See [`StateUpdatesStage`] and [`ReceiptEnvelopeStage`] for reference implementations.
pub trait MigrationStage {
    /// A unique, human-readable identifier for this stage.
    ///
    /// Used as the checkpoint key in
    /// [`MigrationCheckpoints`](crate::tables::MigrationCheckpoints) and as the label in the
    /// progress bar printed to stderr.
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

    /// Returns the total number of items that need migration (for progress reporting).
    ///
    /// The pipeline uses this value together with the current checkpoint position to compute
    /// remaining work and size the progress bar. Return 0 if there is nothing to migrate.
    fn total_items(&self, db: &Db) -> Result<u64, MigrationError>;

    /// Process one batch of up to [`BATCH_SIZE`] items starting from `start_key`.
    ///
    /// All reads and writes **must** go through the provided write transaction `tx`.
    /// Do **not** commit or abort the transaction — the pipeline handles that along with
    /// checkpoint management.
    ///
    /// Return `Some(next_key)` if more items remain after this batch, or `None` when the
    /// stage is complete.
    fn process_batch(&self, tx: &TxRW, start_key: u64) -> Result<Option<u64>, MigrationError>;
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

    /// Returns `true` if the database version is below any stage's threshold.
    fn stage_needed(&self, stage: &dyn MigrationStage) -> bool {
        self.db.version() < stage.threshold_version()
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

    /// Drives a single stage through its batch loop with checkpoint management.
    fn run_stage(&self, stage: &dyn MigrationStage) -> Result<(), MigrationError> {
        let db = self.db;
        let id = stage.id();

        let total = stage.total_items(db)?;
        if total == 0 {
            return Ok(());
        }

        // Resume from the last checkpoint if one exists.
        let checkpoint = db.view(|tx| tx.get::<tables::MigrationCheckpoints>(id.to_string()))?;
        let start_key = checkpoint.map(|cp| cp.next_key).unwrap_or(0);

        if start_key >= total {
            // Already complete — clean up the stale checkpoint.
            db.update(|tx| {
                tx.delete::<tables::MigrationCheckpoints>(id.to_string(), None)?;
                Ok(())
            })?;
            return Ok(());
        }

        let remaining = total - start_key;

        let pb = ProgressBar::new(remaining);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&format!(
                    "[Migrating] \x1b[1;33m{id}\x1b[0m {{bar:40.cyan/blue}} {{pos}}/{{len}} \
                     [{{elapsed_precise}}] {{per_sec}}"
                ))
                .expect("valid format"),
        );

        let mut current_key = start_key;
        loop {
            let tx = db.tx_mut()?;
            let next = stage.process_batch(&tx, current_key)?;

            match next {
                Some(next_key) => {
                    // Intermediate batch — persist checkpoint atomically with data.
                    tx.put::<tables::MigrationCheckpoints>(
                        id.to_string(),
                        MigrationCheckpoint { next_key },
                    )?;
                    tx.commit()?;
                    pb.set_position(next_key - start_key);
                    current_key = next_key;
                }
                None => {
                    // Final batch — remove the checkpoint.
                    tx.delete::<tables::MigrationCheckpoints>(id.to_string(), None)?;
                    tx.commit()?;
                    pb.set_position(remaining);
                    break;
                }
            }
        }

        pb.finish();

        Ok(())
    }
}
