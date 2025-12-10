// Tests for StateTrie stage
//
// Note: The detailed mock-based tests that were here previously tested the state root
// verification logic using mock providers. Since we moved to concrete types (DbProviderFactory),
// these tests would need to be rewritten as integration tests with real data.
//
// The stage itself is tested implicitly through the pipeline integration tests.

use katana_db::abstraction::{Database, DbDupSortCursor, DbTx};
use katana_db::tables;
use katana_db::trie::{SnapshotTrieDb, TrieDbMut};
use katana_primitives::block::BlockNumber;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::DbProviderFactory;
use katana_stage::trie::StateTrie;
use katana_stage::{PruneInput, PruningMode, Stage};
use katana_tasks::TaskManager;
use katana_trie::{ClassesTrie, ContractsTrie, StoragesTrie};

/// Test that the StateTrie stage can be constructed with DbProviderFactory.
#[tokio::test]
async fn can_construct_state_trie_stage() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let _stage = StateTrie::new(provider, task_manager.task_spawner());
}

// ============================================================================
// StateTrie::prune Tests
// ============================================================================

/// Helper to create trie snapshots for testing.
/// Creates snapshots for ClassesTrie, ContractsTrie, and StoragesTrie at given block numbers.
fn create_trie_snapshots(provider: &DbProviderFactory, blocks: &[BlockNumber]) {
    let tx = provider.db().tx_mut().expect("failed to create tx");

    // Create ClassesTrie snapshots
    {
        let mut trie = ClassesTrie::new(TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone()));

        for &block in blocks {
            // Insert unique values for each block
            for i in 0u64..10 {
                let key = Felt::from(block * 1000 + i);
                let value = Felt::from(block * 10000 + i);
                trie.insert(key, value);
            }
            trie.commit(block);
        }
    }

    // Create ContractsTrie snapshots
    {
        let mut trie = ContractsTrie::new(TrieDbMut::<tables::ContractsTrie, _>::new(tx.clone()));

        for &block in blocks {
            for i in 0u64..10 {
                let address = ContractAddress::from(Felt::from(block * 1000 + i));
                let state_hash = Felt::from(block * 10000 + i);
                trie.insert(address, state_hash);
            }
            trie.commit(block);
        }
    }

    // Create StoragesTrie snapshots
    {
        let address = ContractAddress::from(Felt::from(0x1234u64));
        let mut trie =
            StoragesTrie::new(TrieDbMut::<tables::StoragesTrie, _>::new(tx.clone()), address);

        for &block in blocks {
            for i in 0u64..10 {
                let key = Felt::from(block * 1000 + i);
                let value = Felt::from(block * 10000 + i);
                trie.insert(key, value);
            }
            trie.commit(block);
        }
    }

    tx.commit().expect("failed to commit tx");
}

/// Helper to check if a snapshot exists by querying the `Tb::History` table.
///
/// Note: There is currently no efficient way to check snapshot existence at the `SnapshotTrieDb`
/// level. We query the underlying `Tb::History` table directly to verify if entries exist for
/// a given block number. This is consistent with the approach documented in
/// [`TrieDbMut::remove_snapshot`](katana_db::trie::TrieDbMut::remove_snapshot).
fn snapshot_exists<Tb: tables::Trie>(provider: &DbProviderFactory, block: BlockNumber) -> bool {
    let tx = provider.db().tx().expect("failed to create tx");
    let mut cursor = tx.cursor_dup::<Tb::History>().expect("failed to create cursor");
    cursor.walk_dup(Some(block), None).expect("failed to walk_dup").is_some()
}

#[tokio::test]
async fn prune_does_not_affect_remaining_snapshot_roots() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());
    let storage_address = ContractAddress::from(Felt::from(0x1234u64));

    // Create snapshots for blocks 0-9
    create_trie_snapshots(&provider, &(0..=9).collect::<Vec<_>>());

    // Get state roots for all tries at blocks that will remain after pruning (blocks 5-9)
    //
    // [(block_number, classes_root, contracts_root, storages_root), ...]
    let roots_before: Vec<_> = {
        let tx = provider.db().tx().expect("failed to create tx");

        (5..=9)
            .map(|block| {
                let classes_root = ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(
                    tx.clone(),
                    block.into(),
                ))
                .root();

                let contracts_root = ContractsTrie::new(
                    SnapshotTrieDb::<tables::ContractsTrie, _>::new(tx.clone(), block.into()),
                )
                .root();

                let storages_root = StoragesTrie::new(
                    SnapshotTrieDb::<tables::StoragesTrie, _>::new(tx.clone(), block.into()),
                    storage_address,
                )
                .root();

                (block, classes_root, contracts_root, storages_root)
            })
            .collect()
    };

    // Prune blocks 0-4 (Full mode with keep=5, tip=9)
    let input = PruneInput::new(9, PruningMode::Full(5), None);
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().pruned_count, 4); // blocks 0, 1, 2, 3

    // Verify state roots for remaining snapshots (blocks 5-9) are unchanged
    let tx = provider.db().tx().expect("failed to create tx");
    for (block, classes_root_before, contracts_root_before, storages_root_before) in roots_before {
        let classes_root_after = ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(
            tx.clone(),
            block.into(),
        ))
        .root();

        let contracts_root_after = ContractsTrie::new(
            SnapshotTrieDb::<tables::ContractsTrie, _>::new(tx.clone(), block.into()),
        )
        .root();

        let storages_root_after = StoragesTrie::new(
            SnapshotTrieDb::<tables::StoragesTrie, _>::new(tx.clone(), block.into()),
            storage_address,
        )
        .root();

        assert_eq!(
            classes_root_before, classes_root_after,
            "ClassesTrie root at block {block} should be unchanged after pruning"
        );
        assert_eq!(
            contracts_root_before, contracts_root_after,
            "ContractsTrie root at block {block} should be unchanged after pruning"
        );
        assert_eq!(
            storages_root_before, storages_root_after,
            "StoragesTrie root at block {block} should be unchanged after pruning"
        );
    }
}

#[tokio::test]
async fn prune_removes_snapshots_in_range() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create snapshots for blocks 0-9
    create_trie_snapshots(&provider, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

    // Verify all snapshots exist
    for block in 0..=9 {
        assert!(
            snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should exist before pruning"
        );
        assert!(
            snapshot_exists::<tables::ContractsTrie>(&provider, block),
            "ContractsTrie snapshot for block {block} should exist before pruning"
        );
        assert!(
            snapshot_exists::<tables::StoragesTrie>(&provider, block),
            "StoragesTrie snapshot for block {block} should exist before pruning"
        );
    }

    // Prune blocks 0-4 (Full mode with keep=5, tip=9)
    let input = PruneInput::new(9, PruningMode::Full(5), None);
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert_eq!(output.pruned_count, 4); // blocks 0, 1, 2, 3

    // Verify blocks 0-3 snapshots are removed
    for block in 0..=3 {
        assert!(
            !snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should be removed after pruning"
        );
        assert!(
            !snapshot_exists::<tables::ContractsTrie>(&provider, block),
            "ContractsTrie snapshot for block {block} should be removed after pruning"
        );
        assert!(
            !snapshot_exists::<tables::StoragesTrie>(&provider, block),
            "StoragesTrie snapshot for block {block} should be removed after pruning"
        );
    }

    // Verify blocks 4-9 snapshots still exist
    for block in 4..=9 {
        assert!(
            snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should still exist after pruning"
        );
        assert!(
            snapshot_exists::<tables::ContractsTrie>(&provider, block),
            "ContractsTrie snapshot for block {block} should still exist after pruning"
        );
        assert!(
            snapshot_exists::<tables::StoragesTrie>(&provider, block),
            "StoragesTrie snapshot for block {block} should still exist after pruning"
        );
    }
}

#[tokio::test]
async fn prune_skips_when_archive_mode() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create snapshots for blocks 0-4
    create_trie_snapshots(&provider, &[0, 1, 2, 3, 4]);

    // Archive mode should not prune anything
    let input = PruneInput::new(4, PruningMode::Archive, None);
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert_eq!(output.pruned_count, 0);

    // All snapshots should still exist
    for block in 0..=4 {
        assert!(
            snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should still exist"
        );
    }
}

#[tokio::test]
async fn prune_skips_when_already_caught_up() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create snapshots for blocks 5-9 (simulating already-pruned state)
    create_trie_snapshots(&provider, &[5, 6, 7, 8, 9]);

    // Full mode with keep=5, tip=9, last_pruned=4
    // Prune target is 9-5=4, start is 4+1=5, so range 5..4 is empty
    let input = PruneInput::new(9, PruningMode::Full(5), Some(4));
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert_eq!(output.pruned_count, 0);

    // All remaining snapshots should still exist
    for block in 5..=9 {
        assert!(
            snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should still exist"
        );
    }
}

#[tokio::test]
async fn prune_uses_checkpoint_for_incremental_pruning() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create snapshots for blocks 0-14
    create_trie_snapshots(&provider, &(0..=14).collect::<Vec<_>>());

    // First prune: tip=9, keep=5, no previous prune
    // Should prune blocks 0-3 (range 0..4)
    let input = PruneInput::new(9, PruningMode::Full(5), None);
    let result = stage.prune(&input).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().pruned_count, 4);

    // Verify blocks 0-3 are pruned for all trie types
    for block in 0..=3 {
        assert!(
            !snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should be pruned"
        );
        assert!(
            !snapshot_exists::<tables::ContractsTrie>(&provider, block),
            "ContractsTrie snapshot for block {block} should be pruned"
        );
        assert!(
            !snapshot_exists::<tables::StoragesTrie>(&provider, block),
            "StoragesTrie snapshot for block {block} should be pruned"
        );
    }

    // Second prune: tip=14, keep=5, last_pruned=3
    // Should prune blocks 4-8 (range 4..9)
    let input = PruneInput::new(14, PruningMode::Full(5), Some(3));
    let result = stage.prune(&input).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().pruned_count, 5);

    // Verify blocks 4-8 are pruned for all trie types
    for block in 4..=8 {
        assert!(
            !snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should be pruned"
        );
        assert!(
            !snapshot_exists::<tables::ContractsTrie>(&provider, block),
            "ContractsTrie snapshot for block {block} should be pruned"
        );
        assert!(
            !snapshot_exists::<tables::StoragesTrie>(&provider, block),
            "StoragesTrie snapshot for block {block} should be pruned"
        );
    }

    // Verify blocks 9-14 still exist for all trie types
    for block in 9..=14 {
        assert!(
            snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should still exist"
        );
        assert!(
            snapshot_exists::<tables::ContractsTrie>(&provider, block),
            "ContractsTrie snapshot for block {block} should still exist"
        );
        assert!(
            snapshot_exists::<tables::StoragesTrie>(&provider, block),
            "StoragesTrie snapshot for block {block} should still exist"
        );
    }
}

#[tokio::test]
async fn prune_minimal_mode_keeps_only_latest() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create snapshots for blocks 0-9
    create_trie_snapshots(&provider, &(0..=9).collect::<Vec<_>>());

    // Minimal mode: prune everything except tip-1
    // tip=9 -> prune 0..8
    let input = PruneInput::new(9, PruningMode::Minimal, None);
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().pruned_count, 8);

    // Verify blocks 0-7 are pruned
    for block in 0..=7 {
        assert!(
            !snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should be pruned"
        );
        assert!(
            !snapshot_exists::<tables::ContractsTrie>(&provider, block),
            "ContractsTrie snapshot for block {block} should be pruned"
        );
        assert!(
            !snapshot_exists::<tables::StoragesTrie>(&provider, block),
            "StoragesTrie snapshot for block {block} should be pruned"
        );
    }

    // Verify blocks 8-9 still exist
    for block in 8..=9 {
        assert!(
            snapshot_exists::<tables::ClassesTrie>(&provider, block),
            "ClassesTrie snapshot for block {block} should still exist"
        );
        assert!(
            snapshot_exists::<tables::ContractsTrie>(&provider, block),
            "ContractsTrie snapshot for block {block} should still exist"
        );
        assert!(
            snapshot_exists::<tables::StoragesTrie>(&provider, block),
            "StoragesTrie snapshot for block {block} should still exist"
        );
    }
}

#[tokio::test]
async fn prune_handles_empty_range_gracefully() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create snapshots for blocks 0-4
    create_trie_snapshots(&provider, &[0, 1, 2, 3, 4]);

    // Full mode with keep=10, tip=4 - nothing to prune
    let input = PruneInput::new(4, PruningMode::Full(10), None);
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().pruned_count, 0);

    // All snapshots should still exist
    for block in 0..=4 {
        assert!(snapshot_exists::<tables::ClassesTrie>(&provider, block));
    }
}

#[tokio::test]
async fn prune_handles_nonexistent_snapshots_gracefully() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create snapshots only for blocks 5-9 (blocks 0-4 don't exist)
    create_trie_snapshots(&provider, &[5, 6, 7, 8, 9]);

    // Try to prune blocks 0-4 which don't have snapshots
    let input = PruneInput::new(9, PruningMode::Full(5), None);
    let result = stage.prune(&input).await;

    // Should succeed even though blocks 0-3 don't have snapshots
    assert!(result.is_ok());
    // Still counts as "pruned" even if no data existed
    assert_eq!(result.unwrap().pruned_count, 4);

    // Existing snapshots should still be intact
    for block in 5..=9 {
        assert!(snapshot_exists::<tables::ClassesTrie>(&provider, block));
    }
}
