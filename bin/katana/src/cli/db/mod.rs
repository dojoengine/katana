use std::path::{self};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::Table;
use katana_db::mdbx::{DbEnv, DbEnvKind};
use katana_db::version::get_db_version;

mod prune;
mod stats;
mod version;

#[derive(Debug, Args)]
pub struct DbArgs {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Retrieves database statistics
    Stats(stats::StatsArgs),

    /// Shows database version information
    Version(version::VersionArgs),

    /// Prune historical trie data.
    Prune(prune::PruneArgs),
}

impl DbArgs {
    pub fn execute(self) -> Result<()> {
        match self.commands {
            Commands::Prune(args) => args.execute(),
            Commands::Stats(args) => args.execute(),
            Commands::Version(args) => args.execute(),
        }
    }
}

/// Open the database at `path` in read-only mode.
///
/// The path is expanded and resolved to an absolute path before opening the database for clearer
/// error messages.
pub fn open_db_ro(path: &str) -> Result<DbEnv> {
    let path = path::absolute(shellexpand::full(path)?.into_owned())?;
    let version = get_db_version(&path).with_context(|| {
        format!("Reading database version from path {}", path.display())
    })?;
    DbEnv::open(&path, DbEnvKind::RO, version).with_context(|| {
        format!("Opening database file in read-only mode at path {}", path.display())
    })
}

pub fn open_db_rw(path: &str) -> Result<DbEnv> {
    let path = path::absolute(shellexpand::full(path)?.into_owned())?;
    let version = get_db_version(&path).with_context(|| {
        format!("Reading database version from path {}", path.display())
    })?;
    katana_db::open_db(path, version)
}

/// Create a table with the default UTF-8 full border and rounded corners.
fn table() -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
    table
}
