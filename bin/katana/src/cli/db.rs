use std::path::{self};

use anyhow::{ensure, Context, Result};
use clap::{Args, Subcommand};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::Table;
use katana_db::abstraction::{
    Database, DbCursor, DbDupSortCursor, DbDupSortCursorMut, DbTx, DbTxMut,
};
use katana_db::mdbx::{DbEnv, DbEnvKind};
use katana_db::models::list::IntegerSet;
use katana_db::models::trie::{TrieDatabaseKey, TrieHistoryEntry};
use katana_db::tables::{
    ClassesTrieChangeSet, ClassesTrieHistory, ContractsTrieChangeSet, ContractsTrieHistory,
    DupSort, Headers, StoragesTrieChangeSet, StoragesTrieHistory, Table as DbTable, NUM_TABLES,
};
use katana_db::version::{get_db_version, CURRENT_DB_VERSION};
use katana_primitives::block::BlockNumber;

/// Create a human-readable byte unit string (eg. 16.00 KiB)
macro_rules! byte_unit {
    ($size:expr) => {
        format!(
            "{:.2}",
            byte_unit::Byte::from_u64($size as u64)
                .get_appropriate_unit(byte_unit::UnitType::Binary)
        )
    };
}

#[derive(Args)]
pub struct DbArgs {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Retrieves database statistics
    Stats {
        /// Path to the database directory.
        #[arg(short, long)]
        #[arg(default_value = "~/.katana/db")]
        path: String,
    },

    /// Shows database version information
    Version {
        /// Path to the database directory.
        ///
        /// If not provided, the current database version is displayed.
        #[arg(short, long)]
        path: Option<String>,
    },

    #[command(about = "Prune historical trie data")]
    Prune(PruneArgs),
}

#[derive(Args)]
struct PruneArgs {
    /// Path to the database directory.
    #[arg(short, long)]
    #[arg(default_value = "~/.katana/db")]
    path: String,

    #[command(subcommand)]
    mode: PruneMode,
}

#[derive(Subcommand)]
enum PruneMode {
    #[command(about = "Keep only the latest trie state (remove all historical data)")]
    Latest,
    #[command(about = "Keep only the last N blocks of historical data")]
    KeepLast { blocks: u64 },
}

impl DbArgs {
    pub(crate) fn execute(self) -> Result<()> {
        match self.commands {
            Commands::Version { path } => {
                println!("current version: {CURRENT_DB_VERSION}");

                if let Some(path) = path {
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
            }

            Commands::Stats { path } => {
                let db = open_db_ro(&path)?;
                let stats = db.stats()?;

                let mut table = table();
                let mut rows = Vec::with_capacity(NUM_TABLES);
                // total size of all tables (incl. freelist)
                let mut total_size = 0;

                table.set_header(vec![
                    "Table",
                    "Entries",
                    "Depth",
                    "Branch Pages",
                    "Leaf Pages",
                    "Overflow Pages",
                    "Size",
                ]);

                // page size is equal across all tables, so we can just get it from the first table
                // and use it to calculate for the freelist table.
                let mut pagesize: usize = 0;

                for (name, stat) in stats.table_stats().iter() {
                    let entries = stat.entries();
                    let depth = stat.depth();
                    let branch_pages = stat.branch_pages();
                    let leaf_pages = stat.leaf_pages();
                    let overflow_pages = stat.overflow_pages();
                    let size = stat.total_size();

                    rows.push(vec![
                        name.to_string(),
                        entries.to_string(),
                        depth.to_string(),
                        branch_pages.to_string(),
                        leaf_pages.to_string(),
                        overflow_pages.to_string(),
                        byte_unit!(size),
                    ]);

                    // increment the size of all tables
                    total_size += size;

                    if pagesize == 0 {
                        pagesize = stat.page_size() as usize;
                    }
                }

                // sort the rows by the table name
                rows.sort_by(|a, b| a[0].cmp(&b[0]));
                table.add_rows(rows);

                // add special row for the freelist table
                let freelist_size = stats.freelist() * pagesize;
                total_size += freelist_size;

                table.add_row(vec![
                    "Freelist".to_string(),
                    stats.freelist().to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    byte_unit!(freelist_size),
                ]);

                // add the last row for the total size
                table.add_row(vec![
                    "Total Size".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    byte_unit!(total_size),
                ]);

                println!("{table}");
            }

            Commands::Prune(PruneArgs { path, mode }) => {
                prune_database(&path, mode)?;
            }
        }

        Ok(())
    }
}

/// Open the database at `path` in read-only mode.
///
/// The path is expanded and resolved to an absolute path before opening the database for clearer
/// error messages.
fn open_db_ro(path: &str) -> Result<DbEnv> {
    let path = path::absolute(shellexpand::full(path)?.into_owned())?;
    DbEnv::open(&path, DbEnvKind::RO).with_context(|| {
        format!("Opening database file in read-only mode at path {}", path.display())
    })
}

fn prune_database(db_path: &str, mode: PruneMode) -> Result<()> {
    let db = open_db_rw(db_path)?;
    let tx = db.tx_mut().context("Failed to create write transaction")?;

    let latest_block = get_latest_block_number(&tx)?;

    match mode {
        PruneMode::Latest => {
            println!("Pruning all historical trie data...");
            prune_all_history(&tx)?;
        }
        PruneMode::KeepLast { blocks } => {
            if blocks == 0 {
                return Err(anyhow::anyhow!("Number of blocks to keep must be greater than 0"));
            }
            if blocks > latest_block {
                println!(
                    "Warning: Requested to keep {} blocks, but only {} blocks exist",
                    blocks, latest_block
                );
                return Ok(());
            }
            println!("Pruning historical data, keeping last {} blocks...", blocks);
            prune_keep_last_n(&tx, latest_block, blocks)?;
        }
    }

    tx.commit().context("Failed to commit pruning transaction")?;
    println!("Database pruning completed successfully");
    Ok(())
}

fn get_latest_block_number<Tx: DbTx>(tx: &Tx) -> Result<BlockNumber> {
    let mut cursor = tx.cursor::<Headers>()?;
    if let Some((block_num, _)) = cursor.last()? {
        Ok(block_num)
    } else {
        Ok(0)
    }
}

fn prune_all_history<Tx: DbTxMut>(tx: &Tx) -> Result<()> {
    tx.clear::<ClassesTrieHistory>()?;
    tx.clear::<ContractsTrieHistory>()?;
    tx.clear::<StoragesTrieHistory>()?;

    tx.clear::<ClassesTrieChangeSet>()?;
    tx.clear::<ContractsTrieChangeSet>()?;
    tx.clear::<StoragesTrieChangeSet>()?;

    println!("Cleared all historical trie data");
    Ok(())
}

fn prune_keep_last_n<Tx: DbTxMut>(
    tx: &Tx,
    latest_block: BlockNumber,
    keep_blocks: u64,
) -> Result<()> {
    let cutoff_block = latest_block.saturating_sub(keep_blocks);

    if cutoff_block == 0 {
        println!("No blocks to prune");
        return Ok(());
    }

    prune_history_table::<ClassesTrieHistory, _>(tx, cutoff_block)?;
    prune_history_table::<ContractsTrieHistory, _>(tx, cutoff_block)?;
    prune_history_table::<StoragesTrieHistory, _>(tx, cutoff_block)?;

    prune_changeset_table::<ClassesTrieChangeSet, _>(tx, cutoff_block)?;
    prune_changeset_table::<ContractsTrieChangeSet, _>(tx, cutoff_block)?;
    prune_changeset_table::<StoragesTrieChangeSet, _>(tx, cutoff_block)?;

    println!("Pruned historical data for blocks 0 to {}", cutoff_block);
    Ok(())
}

fn prune_history_table<T, Tx>(tx: &Tx, cutoff_block: BlockNumber) -> Result<()>
where
    T: DupSort<Key = BlockNumber, SubKey = TrieDatabaseKey, Value = TrieHistoryEntry>,
    Tx: DbTxMut,
{
    let mut cursor = tx.cursor_dup_mut::<T>()?;
    let mut blocks_to_delete = Vec::new();

    if let Some((block, _)) = cursor.first()? {
        let mut current_block = block;
        while current_block <= cutoff_block {
            blocks_to_delete.push(current_block);
            if let Some((next_block, _)) = cursor.next_no_dup()? {
                current_block = next_block;
            } else {
                break;
            }
        }
    }

    for block in blocks_to_delete {
        if cursor.seek(block)?.is_some() {
            cursor.delete_current_duplicates()?;
        }
    }

    Ok(())
}

fn prune_changeset_table<T, Tx>(tx: &Tx, cutoff_block: BlockNumber) -> Result<()>
where
    T: DbTable<Key = TrieDatabaseKey, Value = IntegerSet>,
    Tx: DbTxMut,
{
    let mut cursor = tx.cursor_mut::<T>()?;
    let mut keys_to_update = Vec::new();

    let walker = cursor.walk(None)?;
    for entry in walker {
        let (key, block_list) = entry?;
        let mut filtered_blocks = IntegerSet::new();
        let mut has_changes = false;

        let mut current_rank = 0;
        while let Some(block) = block_list.select(current_rank) {
            if block > cutoff_block {
                filtered_blocks.insert(block);
            } else {
                has_changes = true;
            }
            current_rank += 1;
        }

        if has_changes {
            if filtered_blocks.select(0).is_none() {
                keys_to_update.push((key, None));
            } else {
                keys_to_update.push((key, Some(filtered_blocks)));
            }
        }
    }

    for (key, maybe_block_list) in keys_to_update {
        if let Some(block_list) = maybe_block_list {
            tx.put::<T>(key, block_list)?;
        } else {
            tx.delete::<T>(key, None)?;
        }
    }

    Ok(())
}

fn open_db_rw(path: &str) -> Result<DbEnv> {
    let path = path::absolute(shellexpand::full(path)?.into_owned())?;
    DbEnv::open(&path, DbEnvKind::RW).with_context(|| {
        format!("Opening database file in read-write mode at path {}", path.display())
    })
}

/// Create a table with the default UTF-8 full border and rounded corners.
fn table() -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
    table
}
