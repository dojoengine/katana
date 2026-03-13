use std::path::PathBuf;

/// Database configurations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DbConfig {
    /// The path to the database directory.
    pub dir: Option<PathBuf>,

    /// Whether to run database migrations on startup.
    ///
    /// When set to `true`, the node will attempt to migrate the existing database to the latest
    /// schema *if* a migration is needed. When `false` (the default), the node will exit with
    /// an error if a migration is required.
    pub migrate: bool,
}
