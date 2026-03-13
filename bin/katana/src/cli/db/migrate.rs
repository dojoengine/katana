use anyhow::Result;
use clap::Args;
use katana_db::migration::Migration;

use super::open_db_rw;

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
        let migration = Migration::new(&db);

        if !migration.is_needed() {
            println!("Database is up to date. No migration needed.");
            return Ok(());
        }

        println!("Running database migration...");
        migration.run()?;
        println!("Database migration completed successfully.");

        Ok(())
    }
}
