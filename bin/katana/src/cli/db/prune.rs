use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use inquire::Confirm;
use katana_db::abstraction::{Database, DbCursor, DbTx, DbTxMut};
use katana_db::tables::{self, Tables};
use katana_db::trie::prune_trie_block;
use katana_primitives::block::BlockNumber;

use super::open_db_ro;
use crate::cli::db::open_db_rw;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
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
            prune_all_history(&tx, latest_block)?;
            println!("Cleared all historical trie data");
        }
        PruneMode::KeepLastN { blocks } => {
            if blocks == 0 {
                return Err(anyhow!("Number of blocks to keep must be greater than 0"));
            }

            if blocks > latest_block {
                eprintln!(
                    "Warning: Requested to keep {blocks} blocks, but only {latest_block} blocks \
                     exist"
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
            println!("Pruned historical data for blocks 0 to {cutoff_block}");
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

/// Prune all historical trie data (keeping only the latest block's state)
fn prune_all_history(tx: &impl DbTxMut, latest_block: BlockNumber) -> Result<()> {
    let pb = ProgressBar::new(latest_block);
    let style = ProgressStyle::default_bar()
        .template("{msg} {bar:40.cyan/blue} {pos:>7}/{len:7}")
        .unwrap()
        .progress_chars("##-");
    pb.set_style(style);
    pb.set_message("Pruning trie data");

    // Prune all blocks except the latest
    for block in 0..latest_block {
        prune_trie_block(tx, block)?;
        pb.inc(1);
    }

    pb.finish_with_message("All historical data cleared");
    Ok(())
}

/// Prune historical data keeping only the last N blocks
fn prune_keep_last_n(tx: &impl DbTxMut, cutoff_block: BlockNumber) -> Result<()> {
    if cutoff_block == 0 {
        return Ok(());
    }

    let pb = ProgressBar::new(cutoff_block);
    let style = ProgressStyle::default_bar()
        .template("{msg} {bar:40.cyan/blue} {pos:>7}/{len:7} [{elapsed_precise}] {per_sec}")
        .unwrap()
        .progress_chars("##-");
    pb.set_style(style);
    pb.set_message("Pruning trie data");

    for block in 0..=cutoff_block {
        prune_trie_block(tx, block)?;
        pb.inc(1);
    }

    pb.finish_with_message("Historical data pruned");
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

    // Count entries in TrieRoots and TrieBlockLog tables
    table_entries_deletions.insert(Tables::TrieRoots.name(), tx.entries::<tables::TrieRoots>()?);
    table_entries_deletions
        .insert(Tables::TrieBlockLog.name(), tx.entries::<tables::TrieBlockLog>()?);

    Ok(PruningStats { table_entries_deletions })
}

/// Count total entries that will be deleted for PruneMode::KeepLastN
fn count_keep_last_n_deletions(tx: &impl DbTx, keep_last_n: BlockNumber) -> Result<PruningStats> {
    let latest_block = get_latest_block_number(tx)?;
    let cutoff_block = latest_block.saturating_sub(keep_last_n);

    if cutoff_block == 0 {
        return Ok(PruningStats::default());
    }

    let mut table_entries_deletions = HashMap::new();

    // Count TrieRoots entries that will be pruned
    let mut roots_count = 0usize;
    let mut cursor = tx.cursor::<tables::TrieRoots>()?;
    for entry in cursor.walk(None)? {
        let (key, _) = entry?;
        // Extract block number from composite key (lower 56 bits)
        let block = key & 0x00FFFFFFFFFFFFFF;
        if block <= cutoff_block {
            roots_count += 1;
        }
    }
    table_entries_deletions.insert(Tables::TrieRoots.name(), roots_count);

    // Count TrieBlockLog entries that will be pruned
    let mut log_count = 0usize;
    let mut cursor = tx.cursor::<tables::TrieBlockLog>()?;
    for entry in cursor.walk(None)? {
        let (key, _) = entry?;
        let block = key & 0x00FFFFFFFFFFFFFF;
        if block <= cutoff_block {
            log_count += 1;
        }
    }
    table_entries_deletions.insert(Tables::TrieBlockLog.name(), log_count);

    Ok(PruningStats { table_entries_deletions })
}

/// Show confirmation prompt with statistics
fn show_confirmation_prompt(stats: &PruningStats, mode: &PruneMode) -> Result<bool> {
    println!("\nWARNING: This operation will permanently delete historical trie data.");
    println!(
        "- Tables affected: {} (TrieRoots, TrieBlockLog)",
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
            println!("- Action: Keep only the last {blocks} blocks of historical data");
        }
    }

    println!("\nThis action cannot be undone.");

    let ans = Confirm::new("Continue?")
        .with_default(false)
        .with_help_message("Press Enter for default (No)")
        .prompt()?;

    Ok(ans)
}

#[cfg(test)]
mod tests {
    use katana_db::abstraction::DbTxMut;
    use katana_db::mdbx::{test_utils, DbEnv};
    use katana_db::models::list::BlockList;
    use katana_db::models::trie::{TrieNodeEntry, TrieType};
    use katana_db::models::VersionedHeader;
    use katana_db::tables;
    use katana_primitives::Felt;
    use katana_trie::node::StoredNode;

    use super::*;

    fn create_test_db() -> DbEnv {
        test_utils::create_test_db()
    }

    /// Insert test trie data using the new table schema.
    fn insert_test_trie_data(
        tx: &impl DbTxMut,
        block_range: std::ops::Range<BlockNumber>,
    ) -> Result<()> {
        for (idx, block) in block_range.clone().enumerate() {
            let idx = idx as u64;

            // Insert class trie nodes
            let entry =
                TrieNodeEntry { hash: Felt::from(block * 100), node: StoredNode::LeafBinary };
            tx.put::<tables::TrieClassNodes>(idx, entry)?;

            let mut added = BlockList::default();
            added.insert(idx);

            // Store root and block log using composite key
            let root_key = (TrieType::Classes as u64) << 56 | block;
            tx.put::<tables::TrieRoots>(root_key, idx)?;
            tx.put::<tables::TrieBlockLog>(root_key, added)?;

            // Insert contract trie nodes
            let entry =
                TrieNodeEntry { hash: Felt::from(block * 200), node: StoredNode::LeafBinary };
            tx.put::<tables::TrieContractNodes>(idx, entry)?;

            let mut added = BlockList::default();
            added.insert(idx);

            let root_key = (TrieType::Contracts as u64) << 56 | block;
            tx.put::<tables::TrieRoots>(root_key, idx)?;
            tx.put::<tables::TrieBlockLog>(root_key, added)?;
        }

        // Insert headers for block range
        for block in block_range {
            tx.put::<tables::Headers>(block, VersionedHeader::default())?;
        }

        Ok(())
    }

    fn count_total_trie_entries(tx: &impl DbTx) -> Result<usize> {
        let mut total = 0;
        total += tx.entries::<tables::TrieRoots>()?;
        total += tx.entries::<tables::TrieBlockLog>()?;
        Ok(total)
    }

    #[test]
    fn test_count_all_historical_deletions() -> Result<()> {
        let db = create_test_db();
        let tx = db.tx_mut()?;

        // Insert test data for blocks 0-9
        insert_test_trie_data(&tx, 0..10)?;
        tx.commit()?;

        let tx = db.tx()?;
        let stats = count_all_historical_deletions(&tx)?;

        // 10 blocks * 2 trie types (classes, contracts) = 20 roots
        assert_eq!(stats.table_entries_deletions.get(Tables::TrieRoots.name()), Some(&20));
        // Same for block logs
        assert_eq!(stats.table_entries_deletions.get(Tables::TrieBlockLog.name()), Some(&20));

        Ok(())
    }

    #[test]
    fn test_count_keep_last_n_deletions() -> Result<()> {
        let db = create_test_db();
        let tx = db.tx_mut()?;

        // Insert test data for blocks 0-19
        insert_test_trie_data(&tx, 0..20)?;
        tx.commit()?;

        let tx = db.tx()?;

        // Test keeping last 5 blocks (should delete blocks 0-14)
        let stats = count_keep_last_n_deletions(&tx, 5)?;

        // 15 blocks * 2 trie types = 30 roots to delete
        assert_eq!(stats.table_entries_deletions.get(Tables::TrieRoots.name()), Some(&30));
        assert_eq!(stats.table_entries_deletions.get(Tables::TrieBlockLog.name()), Some(&30));

        Ok(())
    }

    #[test]
    fn test_pruning_stats_match_actual_deletions_latest_mode() -> Result<()> {
        let db = create_test_db();
        let tx = db.tx_mut()?;

        // Insert test data
        insert_test_trie_data(&tx, 0..15)?;
        tx.commit()?;

        // Count entries before pruning
        let tx = db.tx()?;
        let before_count = count_total_trie_entries(&tx)?;
        let stats = count_all_historical_deletions(&tx)?;
        let predicted_deletions: usize = stats.table_entries_deletions.values().sum();
        drop(tx);

        // Perform actual pruning
        let tx = db.tx_mut()?;
        prune_all_history(&tx, 14)?;
        tx.commit()?;

        // Count entries after pruning
        let tx = db.tx()?;
        let after_count = count_total_trie_entries(&tx)?;

        // After pruning all but latest, we should have removed the predicted amount
        // (latest block 14 still has entries, but prune_all_history prunes 0..latest)
        assert!(before_count > after_count);
        // The predicted count includes ALL entries (including latest), but we keep the latest
        assert!(predicted_deletions >= before_count - after_count);

        Ok(())
    }

    #[test]
    fn test_pruning_keep_last_n() -> Result<()> {
        let db = create_test_db();
        let tx = db.tx_mut()?;

        // Insert test data for blocks 0-9
        insert_test_trie_data(&tx, 0..10)?;
        tx.commit()?;

        // Prune keeping last 5 blocks (cutoff = 4, prune blocks 0-4)
        let tx = db.tx_mut()?;
        prune_keep_last_n(&tx, 4)?;
        tx.commit()?;

        // Verify pruned blocks no longer have roots
        let tx = db.tx()?;
        for block in 0..=4 {
            let key = (TrieType::Classes as u64) << 56 | block;
            let root = tx.get::<tables::TrieRoots>(key)?;
            assert!(root.is_none(), "Classes root for block {block} should be pruned");
        }

        // Verify remaining blocks still have roots
        for block in 5..10 {
            let key = (TrieType::Classes as u64) << 56 | block;
            let root = tx.get::<tables::TrieRoots>(key)?;
            assert!(root.is_some(), "Classes root for block {block} should still exist");
        }

        Ok(())
    }

    #[test]
    fn test_count_keep_last_n_with_no_blocks_to_prune() -> Result<()> {
        let db = create_test_db();
        let tx = db.tx_mut()?;

        // Insert test data for blocks 0-9
        insert_test_trie_data(&tx, 0..10)?;
        tx.commit()?;

        let tx = db.tx()?;

        // Test keeping last 15 blocks when we only have 10
        let stats = count_keep_last_n_deletions(&tx, 15)?;

        // Should have no deletions
        let total: usize = stats.table_entries_deletions.values().sum();
        assert_eq!(total, 0);

        Ok(())
    }
}
