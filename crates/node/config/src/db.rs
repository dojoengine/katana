use std::path::PathBuf;

pub use katana_db::version::DbOpenMode;

/// Database configurations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DbConfig {
    /// The path to the database directory.
    pub dir: Option<PathBuf>,
    /// How Katana should handle older but still supported database versions.
    pub open_mode: DbOpenMode,
}
