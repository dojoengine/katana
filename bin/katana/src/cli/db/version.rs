use std::path;

use anyhow::{ensure, Result};
use clap::Args;
use katana_db::version::{get_db_version, CURRENT_DB_VERSION};

#[derive(Debug, Args)]
pub struct VersionArgs {
    /// Path to the database directory.
    ///
    /// If not provided, the current database version is displayed.
    #[arg(short, long)]
    pub path: Option<String>,
}

impl VersionArgs {
    pub fn execute(self) -> Result<()> {
        println!("current version: {CURRENT_DB_VERSION}");

        if let Some(path) = self.path {
            let expanded_path = shellexpand::full(&path)?;
            let resolved_path = path::absolute(expanded_path.into_owned())?;

            ensure!(
                resolved_path.exists(),
                "database does not exist at path {}",
                resolved_path.display()
            );

            let version = get_db_version(&resolved_path)?;
            println!("database version: {version}");
        }

        Ok(())
    }
}
