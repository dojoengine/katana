//! Code adapted from Paradigm's [`reth`](https://github.com/paradigmxyz/reth/tree/main/crates/storage/db) DB implementation.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use abstraction::Database;
use anyhow::{anyhow, Context};

pub mod abstraction;
pub mod codecs;
pub mod error;
pub mod mdbx;
pub mod migration;
pub mod models;
pub mod static_files;
pub mod tables;
pub mod trie;

pub mod utils;
pub mod version;

use error::DatabaseError;
use libmdbx::SyncMode;
use mdbx::{DbEnv, DbEnvBuilder};
use static_files::{AnyStore, StaticFiles, StaticFilesBuilder};
use utils::is_database_empty;
use version::{
    create_db_version_file, ensure_version_is_openable, get_db_version, DatabaseVersionError,
    Version, LATEST_DB_VERSION,
};

const GIGABYTE: usize = 1024 * 1024 * 1024;
const TERABYTE: usize = GIGABYTE * 1024;

/// Database handle combining MDBX (authoritative index) with static files (bulk data store).
///
/// MDBX stores pointers ([`StaticFileRef`]) into flat `.dat` files for heavy immutable data
/// (headers, transactions, receipts, traces, state updates), plus all mutable state, indexes,
/// and small values directly. On startup, [`recover_static_files`](Self::recover_static_files)
/// truncates any orphaned static file data to match the MDBX-committed state.
#[derive(Debug, Clone)]
pub struct Db {
    env: DbEnv,
    version: Version,
    static_files: Arc<StaticFiles<AnyStore>>,
}

/// Builder for configuring and creating a [`Db`] instance.
///
/// Replaces the individual constructors (`Db::new`, `Db::in_memory`, etc.) with a
/// unified fluent API that configures both MDBX and static files.
///
/// # Examples
///
/// ```ignore
/// // Production database
/// let db = DbBuilder::new().write().build(path)?;
///
/// // In-memory (tests)
/// let db = DbBuilder::new().in_memory().build_ephemeral()?;
///
/// // Custom static file tuning
/// let db = DbBuilder::new()
///     .write()
///     .static_files(|sf| sf.write_buffer_size(1024 * 1024))
///     .build(path)?;
/// ```
#[derive(Debug)]
pub struct DbBuilder {
    mdbx: DbEnvBuilder,
    static_files: StaticFilesBuilder,
    ephemeral: bool,
}

impl DbBuilder {
    pub fn new() -> Self {
        Self {
            mdbx: DbEnvBuilder::new(),
            static_files: StaticFilesBuilder::new(),
            ephemeral: false,
        }
    }

    /// Open in read-write mode (default is read-only).
    pub fn write(mut self) -> Self {
        self.mdbx = self.mdbx.write();
        self
    }

    /// Set MDBX sync mode.
    pub fn sync(mut self, mode: libmdbx::SyncMode) -> Self {
        self.mdbx = self.mdbx.sync(mode);
        self
    }

    /// Set maximum database size.
    pub fn max_size(mut self, size: usize) -> Self {
        self.mdbx = self.mdbx.max_size(size);
        self
    }

    /// Set database growth step.
    pub fn growth_step(mut self, step: isize) -> Self {
        self.mdbx = self.mdbx.growth_step(step);
        self
    }

    /// Use the page size from an existing database.
    pub fn existing_page_size(mut self) -> Self {
        self.mdbx = self.mdbx.existing_page_size();
        self
    }

    /// Configure static files with a closure.
    pub fn static_files(
        mut self,
        f: impl FnOnce(StaticFilesBuilder) -> StaticFilesBuilder,
    ) -> Self {
        self.static_files = f(self.static_files);
        self
    }

    /// Mark as ephemeral (in-memory MDBX + in-memory static files).
    /// Use `build_ephemeral()` instead of `build()`.
    pub fn in_memory(mut self) -> Self {
        self.ephemeral = true;
        self.static_files = self.static_files.memory();
        self
    }

    /// Build a file-backed database at the given path.
    pub fn build<P: AsRef<Path>>(self, path: P) -> anyhow::Result<Db> {
        let path = path.as_ref();
        let version = Db::resolve_or_initialize_version(path)?;

        let env = self.mdbx.build(path)?;
        env.create_default_tables()?;

        let static_files =
            Arc::new(self.static_files.file(path.join("static")).build().with_context(|| {
                format!("Opening static files at {}", path.join("static").display())
            })?);

        Db::recover_static_files(&env, &static_files)?;

        Ok(Db { env, version, static_files })
    }

    /// Build an ephemeral in-memory database (tests, dev mode).
    pub fn build_ephemeral(self) -> anyhow::Result<Db> {
        let dir = tempfile::Builder::new().disable_cleanup(true).tempdir()?;
        let path = dir.path();

        let version = Db::resolve_or_initialize_version(path)?;

        let env = mdbx::DbEnvBuilder::new()
            .max_size(GIGABYTE * 10)
            .growth_step((GIGABYTE / 2) as isize)
            .sync(SyncMode::UtterlyNoSync)
            .build(path)?;

        env.create_default_tables()?;

        let static_files = Arc::new(StaticFiles::in_memory());

        Ok(Db { env, version, static_files })
    }
}

impl Default for DbBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Db {
    /// Initialize the database at the given path and returning a handle to the its
    /// environment.
    ///
    /// This will create the default tables, if necessary.
    pub fn new<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        DbBuilder::new().write().build(path)
    }

    /// Similar to [`init_db`] but will initialize a temporary database.
    ///
    /// Though it is useful for testing per se, but the initial motivation to implement this
    /// variation of database is to be used as the backend for the in-memory storage
    /// provider. Mainly to avoid having two separate implementations for the in-memory and
    /// persistent db. Simplifying it to using a single solid implementation.
    ///
    /// As such, this database environment will trade off durability for write performance and
    /// shouldn't be used in the case where data persistence is required. For that, use
    /// [`init_db`].
    pub fn in_memory() -> anyhow::Result<Self> {
        DbBuilder::new().in_memory().build_ephemeral()
    }

    /// Opens an existing database at the given `path` with [`SyncMode::UtterlyNoSync`] for
    /// write performance, similar to [`Db::in_memory`] but on an existing path.
    ///
    /// This is intended for test scenarios where a pre-populated database snapshot needs to be
    /// loaded quickly without durability guarantees.
    pub fn open_no_sync<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let version = Self::resolve_existing_version(path)?;

        let env = mdbx::DbEnvBuilder::new()
            .max_size(GIGABYTE * 10)
            .growth_step((GIGABYTE / 2) as isize)
            .sync(SyncMode::UtterlyNoSync)
            .existing_page_size()
            .build(path)?;

        env.create_default_tables()?;

        let static_path = path.join("static");
        let static_files = Arc::new(
            StaticFiles::open_file(&static_path)
                .with_context(|| format!("Opening static files at {}", static_path.display()))?,
        );

        Self::recover_static_files(&env, &static_files)?;

        Ok(Self { env, version, static_files })
    }

    /// Open the database at the given `path` in read-write mode.
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        Self::open_inner(path, false).with_context(|| {
            format!("Opening database in read-write mode at path {}", path.display())
        })
    }

    /// Open the database at the given `path` in read-only mode.
    pub fn open_ro<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        Self::open_inner(path, true).with_context(|| {
            format!("Opening database in read-only mode at path {}", path.display())
        })
    }

    fn open_inner<P: AsRef<Path>>(path: P, read_only: bool) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let version = Self::resolve_existing_version(path)?;

        let builder = DbEnvBuilder::new();
        let env = if read_only { builder.build(path)? } else { builder.write().build(path)? };

        let static_path = path.join("static");
        let static_files = Arc::new(
            StaticFiles::open_file(&static_path)
                .with_context(|| format!("Opening static files at {}", static_path.display()))?,
        );

        // Truncate orphaned static file data to match MDBX-committed state.
        Self::recover_static_files(&env, &static_files)?;

        Ok(Self { env, version, static_files })
    }

    pub fn require_migration(&self) -> bool {
        self.version != LATEST_DB_VERSION
    }

    /// Returns the version of the database.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Returns the path to the directory where the database is located.
    pub fn path(&self) -> &Path {
        self.env.path()
    }

    /// Returns a reference to the static files storage.
    pub fn static_files(&self) -> &Arc<StaticFiles<AnyStore>> {
        &self.static_files
    }

    /// Truncate static files to match the MDBX-committed state.
    ///
    /// After a crash, static files may contain orphaned data beyond what MDBX
    /// committed. This method reads the last committed pointers from MDBX and
    /// truncates each static file column to the exact byte position.
    ///
    /// Must be called after opening MDBX and static files, before any writes.
    fn recover_static_files(
        env: &DbEnv,
        static_files: &StaticFiles<AnyStore>,
    ) -> anyhow::Result<()> {
        use crate::abstraction::{DbCursor, DbTx};

        let tx = env.tx().context("Failed to create read transaction for recovery")?;

        // -- Block-level recovery --
        // Find the last committed block by reading the last Headers entry.
        let last_block = tx.cursor::<tables::Headers>()?.last()?;

        let (block_count, headers_end, state_updates_end) = match last_block {
            Some((block_num, ref header_ref)) => {
                let headers_end = header_ref.byte_end().unwrap_or(0);

                // Read the last BlockStateUpdates pointer.
                let su_end = tx
                    .cursor::<tables::BlockStateUpdates>()?
                    .last()?
                    .and_then(|(_, ref r)| r.byte_end())
                    .unwrap_or(0);

                (block_num + 1, headers_end, su_end)
            }
            None => (0, 0, 0),
        };

        // body_indices_end = 0: BlockBodyIndices is stored in MDBX, not static files.
        static_files
            .truncate_blocks(block_count, headers_end, 0, state_updates_end)
            .context("Failed to truncate block static files during recovery")?;

        // -- Transaction-level recovery --
        // Find the last committed tx by reading the last Transactions entry.
        let last_tx = tx.cursor::<tables::Transactions>()?.last()?;

        let (tx_count, transactions_end, receipts_end, traces_end) = match last_tx {
            Some((tx_num, ref tx_ref)) => {
                let transactions_end = tx_ref.byte_end().unwrap_or(0);

                let receipts_end = tx
                    .cursor::<tables::Receipts>()?
                    .last()?
                    .and_then(|(_, ref r)| r.byte_end())
                    .unwrap_or(0);

                let traces_end = tx
                    .cursor::<tables::TxTraces>()?
                    .last()?
                    .and_then(|(_, ref r)| r.byte_end())
                    .unwrap_or(0);

                (tx_num + 1, transactions_end, receipts_end, traces_end)
            }
            None => (0, 0, 0, 0),
        };

        static_files
            .truncate_transactions(tx_count, transactions_end, receipts_end, traces_end)
            .context("Failed to truncate transaction static files during recovery")?;

        Ok(())
    }

    fn resolve_or_initialize_version(path: &Path) -> anyhow::Result<Version> {
        let version = if is_database_empty(path) {
            fs::create_dir_all(path).with_context(|| {
                format!("Creating database directory at path {}", path.display())
            })?;

            create_db_version_file(path, LATEST_DB_VERSION).with_context(|| {
                format!("Inserting database version file at path {}", path.display())
            })?
        } else {
            match get_db_version(path) {
                Ok(version) => {
                    ensure_version_is_openable(version).map_err(anyhow::Error::from)?;
                    version
                }

                Err(DatabaseVersionError::FileNotFound) => {
                    create_db_version_file(path, LATEST_DB_VERSION).with_context(|| {
                        format!(
                            "No database version file found. Inserting version file at path {}",
                            path.display()
                        )
                    })?
                }

                Err(err) => return Err(anyhow!(err)),
            }
        };

        Ok(version)
    }

    fn resolve_existing_version(path: &Path) -> anyhow::Result<Version> {
        let version = get_db_version(path)
            .with_context(|| format!("Getting database version at path {}", path.display()))?;
        ensure_version_is_openable(version)?;
        Ok(version)
    }
}

/// Main persistent database trait. The database implementation must be transactional.
impl Database for Db {
    type Tx = <DbEnv as Database>::Tx;
    type TxMut = <DbEnv as Database>::TxMut;
    type Stats = <DbEnv as Database>::Stats;

    #[track_caller]
    fn tx(&self) -> Result<Self::Tx, DatabaseError> {
        self.env.tx()
    }

    #[track_caller]
    fn tx_mut(&self) -> Result<Self::TxMut, DatabaseError> {
        self.env.tx_mut()
    }

    fn stats(&self) -> Result<Self::Stats, DatabaseError> {
        self.env.stats()
    }
}

impl katana_metrics::Report for Db {
    fn report(&self) {
        self.env.report()
    }
}

#[cfg(test)]
mod tests {

    use std::fs;

    use crate::version::{
        create_db_version_file, default_version_file_path, get_db_version, Version,
        LATEST_DB_VERSION, MIN_OPENABLE_DB_VERSION,
    };
    use crate::Db;

    #[test]
    fn initialize_db_in_empty_dir() {
        let path = tempfile::tempdir().unwrap();
        Db::new(path.path()).unwrap();

        let version_file = fs::File::open(default_version_file_path(path.path())).unwrap();
        let actual_version = get_db_version(path.path()).unwrap();

        assert!(
            version_file.metadata().unwrap().permissions().readonly(),
            "version file should set to read-only"
        );
        assert_eq!(actual_version, LATEST_DB_VERSION);
    }

    #[test]
    fn initialize_db_in_existing_db_dir() {
        let path = tempfile::tempdir().unwrap();

        Db::new(path.path()).unwrap();
        let version = get_db_version(path.path()).unwrap();

        Db::new(path.path()).unwrap();
        let same_version = get_db_version(path.path()).unwrap();

        assert_eq!(version, same_version);
    }

    #[test]
    fn initialize_db_with_malformed_version_file() {
        let path = tempfile::tempdir().unwrap();
        let version_file_path = default_version_file_path(path.path());
        fs::write(version_file_path, b"malformed").unwrap();

        let err = Db::new(path.path()).unwrap_err();
        assert!(err.to_string().contains("Malformed database version file"));
    }

    #[test]
    fn initialize_db_with_mismatch_version() {
        let path = tempfile::tempdir().unwrap();
        let version_file_path = default_version_file_path(path.path());
        fs::write(version_file_path, 99u32.to_be_bytes()).unwrap();

        let err = Db::new(path.path()).unwrap_err();
        assert!(err.to_string().contains("is not supported"));
    }

    #[test]
    fn initialize_db_with_missing_version_file() {
        let path = tempfile::tempdir().unwrap();
        Db::new(path.path()).unwrap();

        fs::remove_file(default_version_file_path(path.path())).unwrap();

        Db::new(path.path()).unwrap();
        let actual_version = get_db_version(path.path()).unwrap();
        assert_eq!(actual_version, LATEST_DB_VERSION);
    }

    #[test]
    fn open_rejects_version_below_supported_floor() {
        let path = tempfile::tempdir().unwrap();
        Db::new(path.path()).unwrap();

        create_db_version_file(path.path(), Version::new(MIN_OPENABLE_DB_VERSION.value() - 1))
            .unwrap();

        let found = Version::new(MIN_OPENABLE_DB_VERSION.value() - 1);
        let err = Db::open_ro(path.path()).unwrap_err();

        let expected = format!(
            "Database version {found} is not supported. Latest supported version is {latest}, \
             minimum openable version is {minimum_openable}.",
            found = found,
            latest = LATEST_DB_VERSION,
            minimum_openable = MIN_OPENABLE_DB_VERSION,
        );
        let err_msg = format!("{err:#}");
        assert!(err_msg.contains(&expected), "error: {err_msg}");
    }

    #[test]
    #[ignore = "unignore once we actually delete the temp directory"]
    fn ephemeral_db_deletion_on_drop() {
        // Create an ephemeral database
        let db = Db::in_memory().expect("failed to create ephemeral database");
        let dir_path = db.path().to_path_buf();

        // Ensure the directory exists
        assert!(dir_path.exists(), "Database directory should exist");

        // Create a clone of the database to increase the reference count
        let db_clone = db.clone();

        // Drop the original database
        drop(db);

        // Directory should still exist because `db_clone` is still alive
        assert!(
            dir_path.exists(),
            "Database directory should still exist after dropping original reference"
        );

        // Drop the cloned database
        drop(db_clone);

        // Now the directory should be deleted
        assert!(
            !dir_path.exists(),
            "Database directory should be deleted after all references are dropped"
        );
    }
}
