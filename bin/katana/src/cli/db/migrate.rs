use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use katana_cli::utils::prompt_db_migration;
use katana_db::migration::Migration;

use super::open_db_rw;

/// Migrate the database to the latest version.
///
/// Check whether the database at the specified path requires migration. If the
/// database is already up to date, exit with no changes. Otherwise, prompt for
/// confirmation before applying the migration.
#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct MigrateArgs {
    /// Path to the database directory.
    #[arg(short, long)]
    pub path: String,
}

impl MigrateArgs {
    pub fn execute(self) -> Result<()> {
        let db = open_db_rw(&self.path)?;
        let migration = Migration::new_v9(&db);

        if !migration.is_needed() {
            println!("Database is up to date. No migration needed.");
            return Ok(());
        }

        if !prompt_db_migration(&PathBuf::from(&self.path))? {
            eprintln!("Migration cancelled.");
            return Ok(());
        }

        Ok(migration.run()?)
    }
}
