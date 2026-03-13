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

/// A migration stage that can be executed against the database.
pub trait MigrationStage {
    /// A human-readable identifier for this stage (used in log messages).
    fn id(&self) -> &'static str;

    /// The database version below which this stage needs to run.
    fn threshold_version(&self) -> Version;

    /// Returns `true` if this stage needs to be executed for the given database.
    fn is_needed(&self, db: &Db) -> bool {
        db.version() < self.threshold_version()
    }

    /// Executes the migration stage against the database.
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
