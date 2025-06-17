use anyhow::{Context, Result};
use clap::Args;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use katana_db::abstraction::{Database, DbCursor, DbDupSortCursorMut, DbTx, DbTxMut};
use katana_db::error::DatabaseError;
use katana_db::models::list::BlockList;
use katana_db::models::trie::TrieDatabaseKey;
use katana_db::tables::{self};
use katana_primitives::block::BlockNumber;

use crate::cli::db::open_db_rw;

#[derive(Debug, Args)]
pub struct PruneArgs {
    /// Path to the database directory.
    #[arg(short, long)]
    pub path: String,

    /// Keep only the latest trie state (remove all historical data)
    #[arg(long, conflicts_with = "keep_last_n")]
    #[arg(required_unless_present = "keep_last_n")]
    pub latest: bool,

    /// Keep only the last N blocks (since the latest block) of historical data
    #[arg(long = "keep-last")]
    #[arg(required_unless_present = "latest")]
    #[arg(value_name = "COUNT", conflicts_with = "latest")]
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    pub keep_last_n: Option<u64>,
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
        prune_database(&self.path, self.mode())
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

    let m = MultiProgress::new();
    let style = ProgressStyle::default_bar()
        .template("{msg} {bar:40.cyan/blue} {pos:>7}/{len:7} [{elapsed_precise}] {per_sec}")
        .unwrap()
        .progress_chars("##-");

    let total_steps = 6;
    let main_pb = m.add(ProgressBar::new(total_steps));
    main_pb.set_style(style.clone());
    main_pb.set_message("Pruning historical data");

    // Prune history tables
    main_pb.set_message("Pruning classes history");
    prune_history_table::<tables::ClassesTrie>(tx, cutoff_block, &m, &style)?;
    main_pb.inc(1);

    main_pb.set_message("Pruning contracts history");
    prune_history_table::<tables::ContractsTrie>(tx, cutoff_block, &m, &style)?;
    main_pb.inc(1);

    main_pb.set_message("Pruning storages history");
    prune_history_table::<tables::StoragesTrie>(tx, cutoff_block, &m, &style)?;
    main_pb.inc(1);

    // Prune changeset tables
    main_pb.set_message("Pruning classes changesets");
    prune_changeset_table::<tables::ClassesTrie>(tx, cutoff_block, &m, &style)?;
    main_pb.inc(1);

    main_pb.set_message("Pruning contracts changesets");
    prune_changeset_table::<tables::ContractsTrie>(tx, cutoff_block, &m, &style)?;
    main_pb.inc(1);

    main_pb.set_message("Pruning storages changesets");
    prune_changeset_table::<tables::StoragesTrie>(tx, cutoff_block, &m, &style)?;
    main_pb.inc(1);

    main_pb.finish_with_message("Historical data pruned");
    Ok(())
}

/// Prune historical entries for a specific trie type up to the cutoff block
fn prune_history_table<T: tables::Trie>(
    tx: &impl DbTxMut,
    cutoff_block: BlockNumber,
    multi_progress: &MultiProgress,
    style: &ProgressStyle,
) -> Result<()> {
    // Count total blocks to prune first
    let mut cursor = tx.cursor_dup_mut::<T::History>()?;
    let mut blocks_to_prune = 0;

    if let Some((block, _)) = cursor.first()? {
        let mut current_block = block;
        while current_block <= cutoff_block {
            blocks_to_prune += 1;
            if let Some((next_block, _)) = cursor.next()? {
                current_block = next_block;
            } else {
                break;
            }
        }
    }

    if blocks_to_prune == 0 {
        return Ok(());
    }

    // Create progress bar for this table
    let pb = multi_progress.add(ProgressBar::new(blocks_to_prune));
    pb.set_style(style.clone());
    pb.set_message(format!(
        "  Pruning {} history blocks",
        std::any::type_name::<T>().split("::").last().unwrap_or("Unknown")
    ));

    // Reset cursor and prune
    let mut cursor = tx.cursor_dup_mut::<T::History>()?;
    if let Some((block, _)) = cursor.first()? {
        let mut current_block = block;
        while current_block <= cutoff_block {
            cursor.delete_current_duplicates()?;
            pb.inc(1);
            if let Some((next_block, _)) = cursor.next()? {
                current_block = next_block;
            } else {
                break;
            }
        }
    }

    pb.finish_and_clear();
    Ok(())
}

/// Prune the changeset table by removing all entries from the genesis block up to the cutoff block
/// (inclusive). Processes entries in batches to reduce memory usage.
fn prune_changeset_table<T: tables::Trie>(
    tx: &impl DbTxMut,
    cutoff_block: BlockNumber,
    multi_progress: &MultiProgress,
    style: &ProgressStyle,
) -> Result<()> {
    const BATCH_SIZE: usize = 1000; // Process 1000 entries at a time

    // Count total entries first
    let mut cursor = tx.cursor_mut::<T::Changeset>()?;
    let mut total_entries = 0;
    for entry in cursor.walk(None)? {
        let (_, block_list) = entry?;
        // Check if this entry will need processing
        if block_list.iter().any(|block| block <= cutoff_block) {
            total_entries += 1;
        }
    }

    if total_entries == 0 {
        return Ok(());
    }

    // Create progress bar for this table
    let pb = multi_progress.add(ProgressBar::new(total_entries));
    pb.set_style(style.clone());
    pb.set_message(format!(
        "  Pruning {} changesets",
        std::any::type_name::<T>().split("::").last().unwrap_or("Unknown")
    ));

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
            pb.inc(1);
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

    pb.finish_and_clear();
    Ok(())
}
