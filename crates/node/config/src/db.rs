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
    /// ## Modes
    ///
    /// ### [`DbOpenMode::Compat`]
    ///
    /// Accepts any database version in Katana's supported compatibility window, which currently
    /// starts at version 5 (`1.6.0`) and ends at the latest version supported by the current
    /// binary defined by the [`LATEST_DB_VERSION`](katana_db::version::LATEST_DB_VERSION)
    /// constant.
    ///
    /// When opening an _older_ supported database read-only, Katana leaves the stored
    /// `db.version` unchanged. But if it's opened with write access, the `db.version` is
    /// updated to the **latest version** before continuing. That preserves the current binary's
    /// forward-compatibility guarantee, but older Katana binaries are no longer guaranteed to
    /// read the database afterward.
    ///
    /// ### [`DbOpenMode::Strict`]
    ///
    /// Disables that compatibility window and only accepts the latest database version. Any older
    /// or newer version is rejected during startup.
    pub open_mode: DbOpenMode,
}
