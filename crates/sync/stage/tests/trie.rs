// Tests for StateTrie stage
//
// Note: The detailed mock-based tests that were here previously tested the state root
// verification logic using mock providers. Since we moved to concrete types (DbProviderFactory),
// these tests would need to be rewritten as integration tests with real data.
//
// The stage itself is tested implicitly through the pipeline integration tests.

use katana_db::abstraction::{Database, DbTx};
use katana_primitives::block::BlockNumber;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::DbProviderFactory;
use katana_stage::trie::StateTrie;
use katana_stage::{PruneInput, Stage};
use katana_tasks::TaskManager;

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

/// Helper to create trie data for testing.
/// Uses the new trie API to insert data via TrieWriter and persist roots.
fn create_trie_data(provider: &DbProviderFactory, blocks: &[BlockNumber]) {
    use katana_db::models::trie::TrieType;
    use katana_db::tables;
    use katana_db::trie::{next_node_index, persist_trie_update};
    use katana_trie::{ClassesTrie, ContractsTrie, MemStorage};

    let tx = provider.db().tx_mut().expect("failed to create tx");

    for &block in blocks {
        // Create and commit a classes trie
        {
            let storage = MemStorage::new();
            let mut trie = ClassesTrie::empty(storage);

            for i in 0u64..3 {
                let key = Felt::from(block * 1000 + i);
                let value = Felt::from(block * 10000 + i);
                trie.insert(key, value).unwrap();
            }

            let update = trie.commit().unwrap();
            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
                &tx,
                &update,
                block,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

        // Create and commit a contracts trie
        {
            let storage = MemStorage::new();
            let mut trie = ContractsTrie::empty(storage);

            for i in 0u64..3 {
                let address = ContractAddress::from(Felt::from(block * 1000 + i));
                let state_hash = Felt::from(block * 10000 + i);
                trie.insert(address, state_hash).unwrap();
            }

            let update = trie.commit().unwrap();
            let mut next_idx = next_node_index::<tables::TrieContractNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieContractNodes, tables::TrieContractLeaves, _>(
                &tx,
                &update,
                block,
                TrieType::Contracts,
                &mut next_idx,
            )
            .unwrap();
        }
    }

    tx.commit().expect("failed to commit tx");
}

/// Helper to check if trie data exists for a block by querying TrieRoots.
fn trie_root_exists(
    provider: &DbProviderFactory,
    trie_type: katana_db::models::trie::TrieType,
    block: BlockNumber,
) -> bool {
    use katana_db::trie::TrieDbFactory;

    let tx = provider.db().tx().expect("failed to create tx");
    let factory = TrieDbFactory::new(tx);
    factory.get_root_for_proofs(trie_type, block).is_some()
}

#[tokio::test]
async fn prune_removes_trie_data_in_range() {
    use katana_db::models::trie::TrieType;

    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create trie data for blocks 0-9
    create_trie_data(&provider, &(0..=9).collect::<Vec<_>>());

    // Verify all roots exist
    for block in 0..=9 {
        assert!(
            trie_root_exists(&provider, TrieType::Classes, block),
            "ClassesTrie root for block {block} should exist before pruning"
        );
        assert!(
            trie_root_exists(&provider, TrieType::Contracts, block),
            "ContractsTrie root for block {block} should exist before pruning"
        );
    }

    // Prune blocks 0-4 (Full mode with keep=5, tip=9)
    let input = PruneInput::new(9, Some(5), None);
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert_eq!(output.pruned_count, 4); // blocks 0, 1, 2, 3

    // Verify blocks 0-3 roots are removed
    for block in 0..=3 {
        assert!(
            !trie_root_exists(&provider, TrieType::Classes, block),
            "ClassesTrie root for block {block} should be removed after pruning"
        );
        assert!(
            !trie_root_exists(&provider, TrieType::Contracts, block),
            "ContractsTrie root for block {block} should be removed after pruning"
        );
    }

    // Verify blocks 4-9 roots still exist
    for block in 4..=9 {
        assert!(
            trie_root_exists(&provider, TrieType::Classes, block),
            "ClassesTrie root for block {block} should still exist after pruning"
        );
        assert!(
            trie_root_exists(&provider, TrieType::Contracts, block),
            "ContractsTrie root for block {block} should still exist after pruning"
        );
    }
}

#[tokio::test]
async fn prune_skips_when_archive_mode() {
    use katana_db::models::trie::TrieType;

    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create trie data for blocks 0-4
    create_trie_data(&provider, &[0, 1, 2, 3, 4]);

    // Archive mode should not prune anything
    let input = PruneInput::new(4, None, None);
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert_eq!(output.pruned_count, 0);

    // All roots should still exist
    for block in 0..=4 {
        assert!(
            trie_root_exists(&provider, TrieType::Classes, block),
            "ClassesTrie root for block {block} should still exist"
        );
    }
}

#[tokio::test]
async fn prune_skips_when_already_caught_up() {
    use katana_db::models::trie::TrieType;

    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create trie data for blocks 5-9 (blocks 0-4 don't exist)
    create_trie_data(&provider, &[5, 6, 7, 8, 9]);

    // Full mode with keep=5, tip=9, last_pruned=4
    let input = PruneInput::new(9, Some(5), Some(4));
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert_eq!(output.pruned_count, 0);

    // All remaining roots should still exist
    for block in 5..=9 {
        assert!(
            trie_root_exists(&provider, TrieType::Classes, block),
            "ClassesTrie root for block {block} should still exist"
        );
    }
}

#[tokio::test]
async fn prune_handles_empty_range_gracefully() {
    use katana_db::models::trie::TrieType;

    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create trie data for blocks 0-4
    create_trie_data(&provider, &[0, 1, 2, 3, 4]);

    // Distance=10, tip=4 - nothing to prune (tip < distance)
    let input = PruneInput::new(4, Some(10), None);
    let result = stage.prune(&input).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().pruned_count, 0);

    // All roots should still exist
    for block in 0..=4 {
        assert!(trie_root_exists(&provider, TrieType::Classes, block));
    }
}

#[tokio::test]
async fn prune_handles_nonexistent_data_gracefully() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let mut stage = StateTrie::new(provider.clone(), task_manager.task_spawner());

    // Create trie data only for blocks 5-9 (blocks 0-4 don't exist)
    create_trie_data(&provider, &[5, 6, 7, 8, 9]);

    // Try to prune blocks 0-4 which don't have trie data
    let input = PruneInput::new(9, Some(5), None);
    let result = stage.prune(&input).await;

    // Should succeed even though blocks 0-3 don't have data
    assert!(result.is_ok());
    assert_eq!(result.unwrap().pruned_count, 4);
}
