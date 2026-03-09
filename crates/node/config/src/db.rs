use std::path::PathBuf;

pub use katana_db::version::DbOpenMode;

/// Database configurations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DbConfig {
    /// The path to the database directory.
    pub dir: Option<PathBuf>,
    /// Controls how Katana validates the on-disk database version when opening an existing DB.
    ///
    /// This setting only matters for existing databases. New databases are always initialized at
    /// the latest format version supported by the current Katana binary.
    ///
    /// [`DbOpenMode::Compat`] accepts any database version in Katana's supported compatibility
    /// window. When opening an older supported database read-only, Katana leaves the stored
    /// `db.version` unchanged. When opening the same database with write access, Katana
    /// immediately rewrites `db.version` to the latest version before continuing. That preserves
    /// the current binary's forward-compatibility guarantee, but older Katana binaries are no
    /// longer guaranteed to read the database afterward.
    ///
    /// [`DbOpenMode::Strict`] disables that compatibility window and only accepts the latest
    /// database version. Any older or newer version is rejected during startup.
    pub open_mode: DbOpenMode,
}
