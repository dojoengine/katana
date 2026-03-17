//! Code adapted from Paradigm's [`reth`](https://github.com/paradigmxyz/reth/tree/main/crates/storage/db) DB implementation.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::fs;
use std::path::Path;

use abstraction::Database;
use anyhow::{anyhow, Context};

pub mod abstraction;
pub mod codecs;
pub mod error;
pub mod mdbx;
pub mod migration;
pub mod models;
pub mod tables;
pub mod trie;

pub mod utils;
pub mod version;

use error::DatabaseError;
use libmdbx::SyncMode;
use mdbx::{DbEnv, DbEnvBuilder};
use utils::is_database_empty;
use version::{
    create_db_version_file, ensure_version_is_openable, get_db_version, DatabaseVersionError,
    Version, LATEST_DB_VERSION,
};

const GIGABYTE: usize = 1024 * 1024 * 1024;
const TERABYTE: usize = GIGABYTE * 1024;

#[derive(Debug, Clone)]
pub struct Db {
    env: DbEnv,
    version: Version,
}

impl Db {
    /// Initialize the database at the given path and returning a handle to the its
    /// environment.
    ///
    /// This will create the default tables, if necessary.
    pub fn new<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let version = Self::resolve_or_initialize_version(path)?;

        let env = DbEnvBuilder::new().write().build(path)?;
        env.create_default_tables()?;

        Ok(Self { env, version })
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
        let dir = tempfile::Builder::new().disable_cleanup(true).tempdir()?;
        let path = dir.path();

        let version = Self::resolve_or_initialize_version(path)?;

        let env = mdbx::DbEnvBuilder::new()
            .max_size(GIGABYTE * 10)  // 10gb
            .growth_step((GIGABYTE / 2) as isize) // 512mb
            .sync(SyncMode::UtterlyNoSync)
            .build(path)?;

        env.create_default_tables()?;

        Ok(Self { env, version })
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

        Ok(Self { env, version })
    }

    // Open the database at the given `path` in read-write mode.
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        Self::open_inner(path, false).with_context(|| {
            format!("Opening database in read-write mode at path {}", path.display())
        })
    }

    // Open the database at the given `path` in read-only mode.
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

        Ok(Self { env, version })
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
