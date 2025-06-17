use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use clap::Args;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use inquire::Confirm;
use katana_db::abstraction::{
    Database, DbCursor, DbDupSortCursor, DbDupSortCursorMut, DbTx, DbTxMut,
};
use katana_db::error::DatabaseError;
use katana_db::models::list::BlockList;
use katana_db::models::trie::TrieDatabaseKey;
use katana_db::tables::{self};
use katana_primitives::block::BlockNumber;

use super::open_db_ro;
use crate::cli::db::open_db_rw;

#[derive(Debug, Args)]
pub struct PruneArgs {
    /// Path to the database directory.
    #[arg(short, long)]
    pub path: String,

    /// Keep only the latest trie state (remove all historical data).
    #[arg(long, conflicts_with = "keep_last_n")]
    #[arg(required_unless_present = "keep_last_n")]
    pub latest: bool,

    /// Keep only the last N blocks (since the latest block) of historical data.
    #[arg(long = "keep-last")]
    #[arg(required_unless_present = "latest")]
    #[arg(value_name = "COUNT", conflicts_with = "latest")]
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    pub keep_last_n: Option<u64>,

    /// Skip confirmation prompt.
    #[arg(short = 'y')]
    pub skip_confirmation: bool,
}

#[derive(Debug)]
pub enum PruneMode {
    /// Keep only the latest trie state (remove all historical data)
    Latest,

    /// Keep only the last N blocks (since the latest block) of historical data
    KeepLastN { blocks: u64 },
}

impl PruneArgs {
    pub fn execute(self) -> Result<()> {
        let mode = self.mode();

        if !self.skip_confirmation && !self.prompt_confirmation()? {
            println!("Pruning operation cancelled.");
            return Ok(());
        }

        prune_database(&self.path, mode)
    }

    fn mode(&self) -> PruneMode {
        if self.latest {
            PruneMode::Latest
        } else if let Some(blocks) = self.keep_last_n {
            PruneMode::KeepLastN { blocks }
        } else {
            unreachable!("invalid prune mode");
        }
    }

    fn prompt_confirmation(&self) -> Result<bool> {
        let mode = self.mode();
        let stats = self.collect_pruning_stats()?;
        show_confirmation_prompt(&stats, &mode)
    }

    /// Collect statistics about what will be pruned
    fn collect_pruning_stats(&self) -> Result<PruningStats> {
        let mode = self.mode();
        let tx = open_db_ro(&self.path)?.tx().context("Failed to create read transaction")?;

        match mode {
            PruneMode::Latest => count_all_historical_deletions(&tx),
            PruneMode::KeepLastN { blocks } => count_keep_last_n_deletions(&tx, blocks),
        }
    }
}

// If prune mode is KeepLastN and the value is more than the available blocks,
// the operation will be a no-op (no data will be pruned).
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
                return Err(anyhow!("Number of blocks to keep must be greater than 0"));
            }

            if blocks > latest_block {
                eprintln!(
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

            prune_keep_last_n(&tx, cutoff_block)?;
            println!("Pruned historical data for blocks 0 to {}", cutoff_block);
        }
    }

    tx.commit().context("Failed to commit pruning transaction")?;
    println!("Database pruning completed successfully");
    Ok(())
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
    let m = MultiProgress::new();
    let style = ProgressStyle::default_bar()
        .template("{msg} {bar:40.cyan/blue} {pos:>7}/{len:7}")
        .unwrap()
        .progress_chars("##-");

    let total_steps = 6;
    let main_pb = m.add(ProgressBar::new(total_steps));
    main_pb.set_style(style.clone());
    main_pb.set_message("Clearing historical tables");

    // Clear each table and update progress
    let tables = [
        (
            "Classes history",
            Box::new(|| tx.clear::<tables::ClassesTrieHistory>())
                as Box<dyn Fn() -> Result<(), DatabaseError>>,
        ),
        (
            "Contracts history",
            Box::new(|| tx.clear::<tables::ContractsTrieHistory>())
                as Box<dyn Fn() -> Result<(), DatabaseError>>,
        ),
        (
            "Storages history",
            Box::new(|| tx.clear::<tables::StoragesTrieHistory>())
                as Box<dyn Fn() -> Result<(), DatabaseError>>,
        ),
        (
            "Classes changeset",
            Box::new(|| tx.clear::<tables::ClassesTrieChangeSet>())
                as Box<dyn Fn() -> Result<(), DatabaseError>>,
        ),
        (
            "Contracts changeset",
            Box::new(|| tx.clear::<tables::ContractsTrieChangeSet>())
                as Box<dyn Fn() -> Result<(), DatabaseError>>,
        ),
        (
            "Storages changeset",
            Box::new(|| tx.clear::<tables::StoragesTrieChangeSet>())
                as Box<dyn Fn() -> Result<(), DatabaseError>>,
        ),
    ];

    for (name, clear_fn) in tables {
        main_pb.set_message(format!("Clearing {name}"));
        clear_fn()?;
        main_pb.inc(1);
    }

    main_pb.finish_with_message("All historical data cleared");
    Ok(())
}

/// Prune historical data keeping only the last N blocks
fn prune_keep_last_n(tx: &impl DbTxMut, cutoff_block: BlockNumber) -> Result<()> {
    if cutoff_block == 0 {
        return Ok(());
    }

    const TOTAL_STEPS: u64 = 6;
    const PROGRESS_BAR_TEMPLATE: &str =
        "{msg} {bar:40.cyan/blue} {pos:>7}/{len:7} [{elapsed_precise}] {per_sec}";

    let pb = ProgressBar::new(TOTAL_STEPS);
    let style =
        ProgressStyle::default_bar().progress_chars("##-").template(PROGRESS_BAR_TEMPLATE).unwrap();
    pb.set_style(style);

    // Prune history tables ---------------------------------------
    pb.set_message("Pruning classes history");
    prune_history_table::<tables::ClassesTrie>(tx, cutoff_block)?;
    pb.inc(1);

    pb.set_message("Pruning contracts history");
    prune_history_table::<tables::ContractsTrie>(tx, cutoff_block)?;
    pb.inc(1);

    pb.set_message("Pruning storages history");
    prune_history_table::<tables::StoragesTrie>(tx, cutoff_block)?;
    pb.inc(1);

    // Prune changeset tables --------------------------------------
    pb.set_message("Pruning classes changesets");
    prune_changeset_table::<tables::ClassesTrie>(tx, cutoff_block)?;
    pb.inc(1);

    pb.set_message("Pruning contracts changesets");
    prune_changeset_table::<tables::ContractsTrie>(tx, cutoff_block)?;
    pb.inc(1);

    pb.set_message("Pruning storages changesets");
    prune_changeset_table::<tables::StoragesTrie>(tx, cutoff_block)?;
    pb.inc(1);

    pb.finish_with_message("Historical data pruned");
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
/// (inclusive). Processes entries in batches to reduce memory usage.
fn prune_changeset_table<T: tables::Trie>(
    tx: &impl DbTxMut,
    cutoff_block: BlockNumber,
) -> Result<()> {
    const BATCH_SIZE: usize = 1000; // Process 1000 entries at a time

    // List of keys to update/delete.
    //
    // If the block list is empty after pruning, delete the key. Otherwise, update the key with the
    // new block list
    let mut keys: Vec<(TrieDatabaseKey, Option<BlockList>)> = Vec::with_capacity(BATCH_SIZE);
    let mut cursor = tx.cursor_mut::<T::Changeset>()?;

    for entry in cursor.walk(None)? {
        let (key, mut block_list) = entry?;
        let mut has_changes = false;

        let total_blocks_removed = block_list.remove_range(0..=cutoff_block);
        if total_blocks_removed > 0 {
            has_changes = true;
        }

        if has_changes {
            if block_list.select(0).is_none() {
                keys.push((key, None));
            } else {
                keys.push((key, Some(block_list)));
            }
        }

        // Process batch when it reaches BATCH_SIZE
        if keys.len() >= BATCH_SIZE {
            for (key, maybe_block_list) in keys.drain(..) {
                if let Some(block_list) = maybe_block_list {
                    tx.put::<T::Changeset>(key, block_list)?;
                } else {
                    tx.delete::<T::Changeset>(key, None)?;
                }
            }
        }
    }

    // Process any remaining entries in the final batch
    for (key, maybe_block_list) in keys {
        if let Some(block_list) = maybe_block_list {
            tx.put::<T::Changeset>(key, block_list)?;
        } else {
            tx.delete::<T::Changeset>(key, None)?;
        }
    }

    Ok(())
}

/// Structure to hold pruning statistics
#[derive(Debug, Default)]
struct PruningStats {
    /// Total number of table entries that will be deleted in the pruning process mapped
    /// according to their table name.
    pub table_entries_deletions: HashMap<&'static str, usize>,
}

/// Count total entries that will be deleted for PruneMode::Latest
fn count_all_historical_deletions(tx: &impl DbTx) -> Result<PruningStats> {
    let mut table_entries_deletions = HashMap::new();

    // Count all entries in history tables
    table_entries_deletions.insert(
        tables::Tables::ClassesTrieHistory.name(),
        tx.entries::<tables::ClassesTrieHistory>()?,
    );
    table_entries_deletions.insert(
        tables::Tables::ContractsTrieHistory.name(),
        tx.entries::<tables::ContractsTrieHistory>()?,
    );
    table_entries_deletions.insert(
        tables::Tables::StoragesTrieHistory.name(),
        tx.entries::<tables::StoragesTrieHistory>()?,
    );

    // Count all entries in changeset tables
    table_entries_deletions.insert(
        tables::Tables::ClassesTrieChangeSet.name(),
        tx.entries::<tables::ClassesTrieChangeSet>()?,
    );
    table_entries_deletions.insert(
        tables::Tables::ContractsTrieChangeSet.name(),
        tx.entries::<tables::ContractsTrieChangeSet>()?,
    );
    table_entries_deletions.insert(
        tables::Tables::StoragesTrieChangeSet.name(),
        tx.entries::<tables::StoragesTrieChangeSet>()?,
    );

    Ok(PruningStats { table_entries_deletions })
}

/// Count total entries that will be deleted for PruneMode::KeepLastN
fn count_keep_last_n_deletions(tx: &impl DbTx, keep_last_n: BlockNumber) -> Result<PruningStats> {
    let cutoff_block = get_latest_block_number(tx)?.saturating_sub(keep_last_n);

    if cutoff_block == 0 {
        return Ok(PruningStats::default());
    }

    let mut table_entries_deletions = HashMap::new();

    // Count entries in history tables
    table_entries_deletions.insert(
        tables::Tables::ClassesTrieHistory.name(),
        count_history_table_deletions::<tables::ClassesTrie>(tx, cutoff_block)?,
    );
    table_entries_deletions.insert(
        tables::Tables::ContractsTrieHistory.name(),
        count_history_table_deletions::<tables::ContractsTrie>(tx, cutoff_block)?,
    );
    table_entries_deletions.insert(
        tables::Tables::StoragesTrieHistory.name(),
        count_history_table_deletions::<tables::StoragesTrie>(tx, cutoff_block)?,
    );

    // Count entries in changeset tables
    table_entries_deletions.insert(
        tables::Tables::ClassesTrieChangeSet.name(),
        count_changeset_table_deletions::<tables::ClassesTrie>(tx, cutoff_block)?,
    );
    table_entries_deletions.insert(
        tables::Tables::ContractsTrieChangeSet.name(),
        count_changeset_table_deletions::<tables::ContractsTrie>(tx, cutoff_block)?,
    );
    table_entries_deletions.insert(
        tables::Tables::StoragesTrieChangeSet.name(),
        count_changeset_table_deletions::<tables::StoragesTrie>(tx, cutoff_block)?,
    );

    Ok(PruningStats { table_entries_deletions })
}

/// Count historical entries that would be deleted for a specific trie type up to the cutoff block
fn count_history_table_deletions<T: tables::Trie>(
    tx: &impl DbTx,
    cutoff_block: BlockNumber,
) -> Result<usize> {
    let mut count = 0;
    let mut cursor = tx.cursor_dup::<T::History>()?;

    for entry in cursor.walk_dup(None, None)?.unwrap() {
        let (block, ..) = entry?;

        if block <= cutoff_block {
            count += 1;
        }
    }

    Ok(count)
}

/// Count changeset entries that would be deleted by removing blocks up to cutoff_block
fn count_changeset_table_deletions<T: tables::Trie>(
    tx: &impl DbTx,
    cutoff_block: BlockNumber,
) -> Result<usize> {
    let mut delete_count = 0;
    let mut cursor = tx.cursor::<T::Changeset>()?;

    for entry in cursor.walk(None)? {
        let (_, block_list) = entry?;

        // only count if the first block is less than the cutoff block (ie, all the blocks will be
        // pruned)
        if let Some(block_num) = block_list.select(0) {
            if block_num <= cutoff_block {
                delete_count += 1;
            }
        }
    }

    Ok(delete_count)
}

/// Show confirmation prompt with statistics
fn show_confirmation_prompt(stats: &PruningStats, mode: &PruneMode) -> Result<bool> {
    println!("\nWARNING: This operation will permanently delete historical trie data.");
    println!(
        "- Tables affected: {} (ClassesTrieHistory, ContractsTrieHistory, etc.)",
        stats.table_entries_deletions.len()
    );
    println!(
        "- Estimated entries to delete: {}",
        stats.table_entries_deletions.values().sum::<usize>()
    );

    match mode {
        PruneMode::Latest => {
            println!("- Action: Remove ALL historical data, keeping only the latest state");
        }
        PruneMode::KeepLastN { blocks } => {
            println!("- Action: Keep only the last {} blocks of historical data", blocks);
        }
    }

    println!("\nThis action cannot be undone.");

    let ans = Confirm::new("Continue?")
        .with_default(false)
        .with_help_message("Press Enter for default (No)")
        .prompt()?;

    Ok(ans)
}
