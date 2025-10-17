use std::path::{self};

use anyhow::Result;
use clap::{Args, Subcommand};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::Table;

mod prune;
mod stats;
mod trie;
mod version;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct DbArgs {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
enum Commands {
    /// Retrieves database statistics
    Stats(stats::StatsArgs),

    /// Shows database version information
    Version(version::VersionArgs),

    /// Prune historical trie data.
    Prune(prune::PruneArgs),

    /// Inspect trie roots stored in the database.
    Trie(trie::TrieArgs),
}

impl DbArgs {
    pub fn execute(self) -> Result<()> {
        match self.commands {
            Commands::Prune(args) => args.execute(),
            Commands::Stats(args) => args.execute(),
            Commands::Version(args) => args.execute(),
            Commands::Trie(args) => args.execute(),
        }
    }
}

/// Open the database at `path` in read-only mode.
///
/// The path is expanded and resolved to an absolute path before opening the database for clearer
/// error messages.
pub fn open_db_ro(path: &str) -> Result<katana_db::Db> {
    katana_db::Db::open_ro(&path::absolute(shellexpand::full(path)?.into_owned())?)
}

/// Open the database at `path` in read-write mode.
///
/// The path is expanded and resolved to an absolute path before opening the database for clearer
/// error messages.
pub fn open_db_rw(path: &str) -> Result<katana_db::Db> {
    katana_db::Db::open(&path::absolute(shellexpand::full(path)?.into_owned())?)
}

/// Create a table with the default UTF-8 full border and rounded corners.
fn table() -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
    table
}
