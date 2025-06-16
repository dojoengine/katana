use std::path::{self};

use anyhow::{ensure, Context, Result};
use clap::{Args, Subcommand};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::Table;
use katana_db::abstraction::{Database, DbCursor, DbDupSortCursorMut, DbTx, DbTxMut};
use katana_db::mdbx::{DbEnv, DbEnvKind};
use katana_db::models::list::BlockList;
use katana_db::models::trie::TrieDatabaseKey;
use katana_db::tables::{self, NUM_TABLES};
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

#[derive(Debug, Args)]
pub struct DbArgs {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
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

    /// Prune historical trie data.
    Prune(PruneArgs),
}

#[derive(Debug, Args)]
struct PruneArgs {
    /// Path to the database directory.
    #[arg(short, long)]
    path: String,

    #[command(subcommand)]
    mode: PruneMode,
}

#[derive(Debug, Subcommand)]
pub enum PruneMode {
    // Keep only the latest trie state (remove all historical data)
    Latest,

    /// Keep only the last N blocks (since tha latest block) of historical data
    KeepLastN {
        #[arg(value_name = "BLOCKS")]
        #[arg(value_parser = clap::value_parser!(u64).range(1..))]
        blocks: u64,
    },
}

impl DbArgs {
    pub fn execute(self) -> Result<()> {
        match self.commands {
            Commands::Version { path } => {
                display_version(path)?;
            }
            Commands::Stats { path } => {
                display_stats(&path)?;
            }
            Commands::Prune(PruneArgs { path, mode }) => {
                prune_database(&path, mode)?;
            }
        }

        Ok(())
    }
}

/// Display database statistics in a formatted table.
fn display_stats(db_path: &str) -> Result<()> {
    let db = open_db_ro(db_path)?;
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

    Ok(())
}

fn display_version(path: Option<String>) -> Result<()> {
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

    Ok(())
}

fn prune_database(db_path: &str, mode: PruneMode) -> Result<()> {
    let db = open_db_rw(db_path)?;
    let tx = db.tx_mut().context("Failed to create write transaction")?;

    let latest_block = get_latest_block_number(&tx)?;

    match mode {
        PruneMode::Latest => {
            println!("Pruning all historical trie data...");
            prune_all_history(&tx)?;
            println!("Cleared all historical trie data");
        }
        PruneMode::KeepLastN { blocks } => {
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

            let cutoff_block = latest_block.saturating_sub(blocks);
            println!("Pruning historical data, keeping last {blocks} blocks...");

            if cutoff_block == 0 {
                println!("No blocks to prune");
                return Ok(());
            }

            prune_keep_last_n(&tx, latest_block, blocks)?;
            println!("Pruned historical data for blocks 0 to {}", cutoff_block);
        }
    }

    tx.commit().context("Failed to commit pruning transaction")?;
    println!("Database pruning completed successfully");
    Ok(())
}

/// Open the database at `path` in read-only mode.
///
/// The path is expanded and resolved to an absolute path before opening the database for clearer
/// error messages.
pub fn open_db_ro(path: &str) -> Result<DbEnv> {
    let path = path::absolute(shellexpand::full(path)?.into_owned())?;
    DbEnv::open(&path, DbEnvKind::RO).with_context(|| {
        format!("Opening database file in read-only mode at path {}", path.display())
    })
}

pub fn open_db_rw(path: &str) -> Result<DbEnv> {
    let path = path::absolute(shellexpand::full(path)?.into_owned())?;
    katana_db::open_db(path)
}

/// Create a table with the default UTF-8 full border and rounded corners.
fn table() -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
    table
}

/// Get the latest block number from the Headers table
fn get_latest_block_number(tx: &impl DbTx) -> Result<BlockNumber> {
    let mut cursor = tx.cursor::<tables::Headers>()?;
    if let Some((block_num, _)) = cursor.last()? {
        Ok(block_num)
    } else {
        Ok(0)
    }
}

/// Prune all historical trie data (keeping only current state)
fn prune_all_history(tx: &impl DbTxMut) -> Result<()> {
    tx.clear::<tables::ClassesTrieHistory>()?;
    tx.clear::<tables::ContractsTrieHistory>()?;
    tx.clear::<tables::StoragesTrieHistory>()?;

    tx.clear::<tables::ClassesTrieChangeSet>()?;
    tx.clear::<tables::ContractsTrieChangeSet>()?;
    tx.clear::<tables::StoragesTrieChangeSet>()?;

    Ok(())
}

/// Prune historical data keeping only the last N blocks
fn prune_keep_last_n(tx: &impl DbTxMut, latest_block: BlockNumber, keep_blocks: u64) -> Result<()> {
    let cutoff_block = latest_block.saturating_sub(keep_blocks);

    if cutoff_block == 0 {
        return Ok(());
    }

    prune_history_table::<tables::ClassesTrie>(tx, cutoff_block)?;
    prune_history_table::<tables::ContractsTrie>(tx, cutoff_block)?;
    prune_history_table::<tables::StoragesTrie>(tx, cutoff_block)?;

    prune_changeset_table::<tables::ClassesTrie>(tx, cutoff_block)?;
    prune_changeset_table::<tables::ContractsTrie>(tx, cutoff_block)?;
    prune_changeset_table::<tables::StoragesTrie>(tx, cutoff_block)?;

    Ok(())
}

/// Prune historical entries for a specific trie type up to the cutoff block
fn prune_history_table<T: tables::Trie>(
    tx: &impl DbTxMut,
    cutoff_block: BlockNumber,
) -> Result<()> {
    let mut cursor = tx.cursor_dup_mut::<T::History>()?;

    if let Some((block, _)) = cursor.first()? {
        let mut current_block = block;
        while current_block <= cutoff_block {
            cursor.delete_current_duplicates()?;
            if let Some((next_block, _)) = cursor.next()? {
                current_block = next_block;
            } else {
                break;
            }
        }
    }

    Ok(())
}

/// Prune the changeset table by removing all entries from the genesis block up to the cutoff block
/// (inclusive).
fn prune_changeset_table<T: tables::Trie>(
    tx: &impl DbTxMut,
    cutoff_block: BlockNumber,
) -> Result<()> {
    let mut cursor = tx.cursor_mut::<T::Changeset>()?;
    let mut keys_to_update: Vec<(TrieDatabaseKey, Option<BlockList>)> = Vec::new();

    let walker = cursor.walk(None)?;
    for entry in walker {
        let (key, mut block_list) = entry?;
        let mut has_changes = false;

        let total_blocks_removed = block_list.remove_range(0..=cutoff_block);
        if total_blocks_removed > 0 {
            has_changes = true;
        }

        if has_changes {
            if block_list.select(0).is_none() {
                keys_to_update.push((key, None));
            } else {
                keys_to_update.push((key, Some(block_list)));
            }
        }
    }

    for (key, maybe_block_list) in keys_to_update {
        if let Some(block_list) = maybe_block_list {
            tx.put::<T::Changeset>(key, block_list)?;
        } else {
            tx.delete::<T::Changeset>(key, None)?;
        }
    }

    Ok(())
}
