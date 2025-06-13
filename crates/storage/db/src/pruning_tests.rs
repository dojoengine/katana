
use std::collections::HashMap;

use katana_primitives::block::BlockNumber;

use crate::abstraction::{Database, DbCursor, DbDupSortCursor, DbDupSortCursorMut, DbTx, DbTxMut};
use crate::models::list::IntegerSet;
use crate::models::trie::{TrieDatabaseKey, TrieDatabaseKeyType, TrieHistoryEntry};
use crate::tables::{
    ClassesTrie, ClassesTrieChangeSet, ClassesTrieHistory, ContractsTrie, ContractsTrieChangeSet,
    ContractsTrieHistory, StoragesTrie, StoragesTrieChangeSet, StoragesTrieHistory, Table,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdbx::test_utils::create_test_db;

    fn generate_test_key(key_type: TrieDatabaseKeyType, suffix: u8) -> TrieDatabaseKey {
        TrieDatabaseKey {
            r#type: key_type,
            key: vec![suffix, suffix + 1, suffix + 2],
        }
    }

    fn generate_test_value() -> Vec<u8> {
        vec![1, 2, 3, 4, 5]
    }

    fn populate_test_data(
        tx: &impl DbTxMut,
        blocks: &[BlockNumber],
    ) -> anyhow::Result<HashMap<TrieDatabaseKey, Vec<BlockNumber>>> {
        let mut changeset_data = HashMap::new();

        for &block_num in blocks {
            for (key_type, suffix) in [
                (TrieDatabaseKeyType::Trie, 10),
                (TrieDatabaseKeyType::Flat, 20),
                (TrieDatabaseKeyType::TrieLog, 30),
            ] {
                let key = generate_test_key(key_type, suffix + (block_num as u8) % 10);
                let value = generate_test_value();

                let history_entry = TrieHistoryEntry {
                    key: key.clone(),
                    value: value.clone().into(),
                };

                tx.put::<ClassesTrieHistory>(block_num, history_entry.clone())?;
                tx.put::<ContractsTrieHistory>(block_num, history_entry.clone())?;
                tx.put::<StoragesTrieHistory>(block_num, history_entry)?;

                changeset_data
                    .entry(key.clone())
                    .or_insert_with(Vec::new)
                    .push(block_num);
            }
        }

        for (key, block_list) in &changeset_data {
            let mut classes_set = IntegerSet::new();
            let mut contracts_set = IntegerSet::new();
            let mut storages_set = IntegerSet::new();
            
            for &block in block_list {
                classes_set.insert(block);
                contracts_set.insert(block);
                storages_set.insert(block);
            }

            tx.put::<ClassesTrieChangeSet>(key.clone(), classes_set)?;
            tx.put::<ContractsTrieChangeSet>(key.clone(), contracts_set)?;
            tx.put::<StoragesTrieChangeSet>(key.clone(), storages_set)?;
        }

        Ok(changeset_data)
    }

    fn populate_current_trie_state(tx: &impl DbTxMut) -> anyhow::Result<()> {
        let current_key = generate_test_key(TrieDatabaseKeyType::Trie, 100);
        let current_value = generate_test_value();

        tx.put::<ClassesTrie>(current_key.clone(), current_value.clone().into())?;
        tx.put::<ContractsTrie>(current_key.clone(), current_value.clone().into())?;
        tx.put::<StoragesTrie>(current_key, current_value.into())?;

        Ok(())
    }

    fn count_history_entries<T>(tx: &impl DbTx) -> anyhow::Result<usize>
    where
        T: Table,
    {
        let mut cursor = tx.cursor::<T>()?;
        let mut count = 0;
        let walker = cursor.walk(None)?;
        for entry in walker {
            entry?;
            count += 1;
        }
        Ok(count)
    }

    fn count_changeset_entries<T>(tx: &impl DbTx) -> anyhow::Result<usize>
    where
        T: Table,
    {
        let mut cursor = tx.cursor::<T>()?;
        let mut count = 0;
        let walker = cursor.walk(None)?;
        for entry in walker {
            entry?;
            count += 1;
        }
        Ok(count)
    }

    #[test]
    fn test_prune_all_history() -> anyhow::Result<()> {
        let db = create_test_db();
        let tx = db.tx_mut()?;

        let blocks = vec![1, 2, 3, 4, 5];
        populate_test_data(&tx, &blocks)?;
        populate_current_trie_state(&tx)?;

        assert!(count_history_entries::<ClassesTrieHistory>(&tx)? > 0);
        assert!(count_history_entries::<ContractsTrieHistory>(&tx)? > 0);
        assert!(count_history_entries::<StoragesTrieHistory>(&tx)? > 0);
        assert!(count_changeset_entries::<ClassesTrieChangeSet>(&tx)? > 0);
        assert!(count_changeset_entries::<ContractsTrieChangeSet>(&tx)? > 0);
        assert!(count_changeset_entries::<StoragesTrieChangeSet>(&tx)? > 0);

        let classes_trie_count_before = count_changeset_entries::<ClassesTrie>(&tx)?;
        let contracts_trie_count_before = count_changeset_entries::<ContractsTrie>(&tx)?;
        let storages_trie_count_before = count_changeset_entries::<StoragesTrie>(&tx)?;

        tx.clear::<ClassesTrieHistory>()?;
        tx.clear::<ContractsTrieHistory>()?;
        tx.clear::<StoragesTrieHistory>()?;
        tx.clear::<ClassesTrieChangeSet>()?;
        tx.clear::<ContractsTrieChangeSet>()?;
        tx.clear::<StoragesTrieChangeSet>()?;

        assert_eq!(count_history_entries::<ClassesTrieHistory>(&tx)?, 0);
        assert_eq!(count_history_entries::<ContractsTrieHistory>(&tx)?, 0);
        assert_eq!(count_history_entries::<StoragesTrieHistory>(&tx)?, 0);
        assert_eq!(count_changeset_entries::<ClassesTrieChangeSet>(&tx)?, 0);
        assert_eq!(count_changeset_entries::<ContractsTrieChangeSet>(&tx)?, 0);
        assert_eq!(count_changeset_entries::<StoragesTrieChangeSet>(&tx)?, 0);

        assert_eq!(count_changeset_entries::<ClassesTrie>(&tx)?, classes_trie_count_before);
        assert_eq!(count_changeset_entries::<ContractsTrie>(&tx)?, contracts_trie_count_before);
        assert_eq!(count_changeset_entries::<StoragesTrie>(&tx)?, storages_trie_count_before);

        tx.commit()?;
        Ok(())
    }

    #[test]
    fn test_prune_keep_last_n() -> anyhow::Result<()> {
        let db = create_test_db();
        let tx = db.tx_mut()?;

        let blocks: Vec<BlockNumber> = (1..=10).collect();
        let changeset_data = populate_test_data(&tx, &blocks)?;
        populate_current_trie_state(&tx)?;

        let classes_trie_count_before = count_changeset_entries::<ClassesTrie>(&tx)?;
        let contracts_trie_count_before = count_changeset_entries::<ContractsTrie>(&tx)?;
        let storages_trie_count_before = count_changeset_entries::<StoragesTrie>(&tx)?;

        let keep_blocks = 3;
        let latest_block = 10;
        let cutoff_block = latest_block - keep_blocks; // Should be 7

        let mut cursor = tx.cursor_dup_mut::<ClassesTrieHistory>()?;
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

        let mut remaining_blocks = Vec::new();
        let mut cursor = tx.cursor::<ClassesTrieHistory>()?;
        let walker = cursor.walk(None)?;
        for entry in walker {
            let (block, _) = entry?;
            remaining_blocks.push(block);
        }

        assert!(remaining_blocks.iter().all(|&block| block > cutoff_block));
        assert!(remaining_blocks.len() <= (keep_blocks as usize) * 3); // 3 key types per block

        assert_eq!(count_changeset_entries::<ClassesTrie>(&tx)?, classes_trie_count_before);
        assert_eq!(count_changeset_entries::<ContractsTrie>(&tx)?, contracts_trie_count_before);
        assert_eq!(count_changeset_entries::<StoragesTrie>(&tx)?, storages_trie_count_before);

        tx.commit()?;
        Ok(())
    }

    #[test]
    fn test_prune_empty_database() -> anyhow::Result<()> {
        let db = create_test_db();
        let tx = db.tx_mut()?;

        assert_eq!(count_history_entries::<ClassesTrieHistory>(&tx)?, 0);
        assert_eq!(count_changeset_entries::<ClassesTrieChangeSet>(&tx)?, 0);

        tx.clear::<ClassesTrieHistory>()?;
        tx.clear::<ClassesTrieChangeSet>()?;

        assert_eq!(count_history_entries::<ClassesTrieHistory>(&tx)?, 0);
        assert_eq!(count_changeset_entries::<ClassesTrieChangeSet>(&tx)?, 0);

        tx.commit()?;
        Ok(())
    }

    #[test]
    #[cfg(feature = "arbitrary")]
    fn test_arbitrary_key_generation() {
        use arbitrary::{Arbitrary, Unstructured};

        let data = vec![0u8; 1000]; // Provide enough data for arbitrary generation
        let mut u = Unstructured::new(&data);

        for _ in 0..100 {
            let key = TrieDatabaseKey::arbitrary(&mut u).unwrap();
            assert!(key.key.len() <= 256, "Generated key exceeds 256 bytes: {}", key.key.len());
        }
    }
}
