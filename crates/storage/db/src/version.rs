use std::array::TryFromSliceError;
use std::fmt::Display;
use std::fs::{self};
use std::io::{Read, Write};
use std::mem;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Latest on-disk database version written by current Katana.
pub const LATEST_DB_VERSION: Version = Version::new(9);
/// Oldest database version current Katana guarantees it can still open.
pub const MIN_OPENABLE_DB_VERSION: Version = Version::new(5);

/// Name of the version file.
const DB_VERSION_FILE_NAME: &str = "db.version";

#[derive(Debug, thiserror::Error)]
pub enum DatabaseVersionError {
    #[error("Database version file not found.")]
    FileNotFound,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Malformed database version file: {0}")]
    MalformedContent(#[from] TryFromSliceError),

    #[error(
        "Database version {found} is not supported. Latest supported version is {latest}, minimum \
         openable version is {minimum_openable}."
    )]
    IncompatibleVersion { found: Version, latest: Version, minimum_openable: Version },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u32);

impl Version {
    pub const fn new(version: u32) -> Self {
        Version(version)
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Returns `true` if the given database version is openable.
pub fn is_version_openable(version: Version) -> bool {
    version >= MIN_OPENABLE_DB_VERSION && version <= LATEST_DB_VERSION
}

/// Validates that the requested database version is openable.
pub fn ensure_version_is_openable(version: Version) -> Result<(), DatabaseVersionError> {
    if is_version_openable(version) {
        Ok(())
    } else {
        Err(DatabaseVersionError::IncompatibleVersion {
            found: version,
            latest: LATEST_DB_VERSION,
            minimum_openable: MIN_OPENABLE_DB_VERSION,
        })
    }
}

/// Get the version of the database at the given `path`.
pub fn get_db_version(path: impl AsRef<Path>) -> Result<Version, DatabaseVersionError> {
    let path = version_file_path(path.as_ref());

    let mut file = fs::File::open(path).map_err(|_| DatabaseVersionError::FileNotFound)?;
    let mut buf: Vec<u8> = Vec::new();
    file.read_to_end(&mut buf)?;

    let bytes = <[u8; mem::size_of::<u32>()]>::try_from(buf.as_slice())?;
    Ok(Version(u32::from_be_bytes(bytes)))
}

/// Insert a version file at the given `path` with the specified `version`. If the `path` is a
/// directory, the version file will be created inside it. Otherwise, the version file will be
/// created exactly at `path`.
///
/// Ideally the version file should be included in the database directory.
///
/// # Errors
///
/// Will fail if all the directories in `path` has not already been created.
pub(crate) fn write_db_version_file(
    path: impl AsRef<Path>,
    version: Version,
) -> Result<Version, DatabaseVersionError> {
    let path = version_file_path(path.as_ref());

    if path.exists() {
        let mut permissions = fs::metadata(&path)?.permissions();
        if permissions.readonly() {
            set_permissions_writable(&mut permissions);
            fs::set_permissions(&path, permissions)?;
        }
    }

    let mut file = fs::File::create(&path)?;
    file.write_all(&version.0.to_be_bytes()).map_err(DatabaseVersionError::Io)?;

    let mut permissions = file.metadata()?.permissions();
    set_permissions_readonly(&mut permissions);
    file.set_permissions(permissions)?;

    Ok(version)
}

/// Insert a version file for newly-created databases.
pub fn create_db_version_file(
    path: impl AsRef<Path>,
    version: Version,
) -> Result<Version, DatabaseVersionError> {
    write_db_version_file(path, version)
}

pub(super) fn default_version_file_path(path: &Path) -> PathBuf {
    path.join(DB_VERSION_FILE_NAME)
}

fn version_file_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        default_version_file_path(path)
    } else {
        path.to_path_buf()
    }
}

#[cfg(unix)]
fn set_permissions_writable(permissions: &mut fs::Permissions) {
    permissions.set_mode(permissions.mode() | 0o200);
}

#[cfg(not(unix))]
fn set_permissions_writable(permissions: &mut fs::Permissions) {
    permissions.set_readonly(false);
}

#[cfg(unix)]
fn set_permissions_readonly(permissions: &mut fs::Permissions) {
    permissions.set_mode(permissions.mode() & !0o222);
}

#[cfg(not(unix))]
fn set_permissions_readonly(permissions: &mut fs::Permissions) {
    permissions.set_readonly(true);
}

#[cfg(test)]
mod tests {
    use super::{ensure_version_is_openable, Version, LATEST_DB_VERSION, MIN_OPENABLE_DB_VERSION};

    #[test]
    fn test_version_constants() {
        assert_eq!(LATEST_DB_VERSION.value(), 9, "Invalid latest database version");
        assert_eq!(MIN_OPENABLE_DB_VERSION.value(), 5, "Invalid minimum openable database version");
    }

    #[test]
    fn accepts_supported_range() {
        assert!(ensure_version_is_openable(MIN_OPENABLE_DB_VERSION).is_ok());
        assert!(ensure_version_is_openable(LATEST_DB_VERSION).is_ok());
        // Also check a version in between
        for v in MIN_OPENABLE_DB_VERSION.value()..=LATEST_DB_VERSION.value() {
            assert!(ensure_version_is_openable(Version::new(v)).is_ok());
        }
    }

    #[test]
    fn rejects_outside_supported_range() {
        assert!(ensure_version_is_openable(Version::new(4)).is_err());
        assert!(ensure_version_is_openable(Version::new(LATEST_DB_VERSION.value() + 1),).is_err());
    }
}
