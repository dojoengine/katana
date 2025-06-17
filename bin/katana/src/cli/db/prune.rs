use anyhow::{Context, Result};
use clap::Args;
use katana_db::abstraction::{Database, DbCursor, DbDupSortCursorMut, DbTx, DbTxMut};
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
    tx.clear::<tables::ClassesTrieHistory>()?;
    tx.clear::<tables::ContractsTrieHistory>()?;
    tx.clear::<tables::StoragesTrieHistory>()?;

    tx.clear::<tables::ClassesTrieChangeSet>()?;
    tx.clear::<tables::ContractsTrieChangeSet>()?;
    tx.clear::<tables::StoragesTrieChangeSet>()?;

    Ok(())
}

/// Prune historical data keeping only the last N blocks
fn prune_keep_last_n(tx: &impl DbTxMut, cutoff_block: BlockNumber) -> Result<()> {
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
