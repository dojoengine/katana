#[allow(unused)]
mod common;

use std::collections::BTreeMap;

use common::{
    create_provider_with_block_data_only, create_provider_with_block_range,
    state_updates_with_contract_changes,
};
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::{felt, ContractAddress};
use katana_provider::api::state::{
    HistoricalStateRetentionProvider, StateFactoryProvider, StateProvider,
};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::{MutableProvider, ProviderError, ProviderFactory};
use katana_stage::{PruneInput, Stage, StageExecutionInput};
use katana_tasks::TaskManager;

// ---- execute tests ----

/// First sync (from=0) uses the bulk write path. Verify that after executing, all
/// state changes are queryable via both `latest()` and `historical()` providers.
#[tokio::test]
async fn execute_first_sync_indexes_state_correctly() {
    let addr = ContractAddress::from(felt!("0x1"));
    let class_hash = felt!("0xAA");
    let nonce = felt!("0x3");
    let storage_key = felt!("0x10");
    let storage_value = felt!("0x20");

    let mut updates = BTreeMap::new();
    updates.insert(
        1,
        state_updates_with_contract_changes(addr, class_hash, nonce, storage_key, storage_value),
    );

    // Only block data stored — no state history yet.
    let provider = create_provider_with_block_data_only(0..=3, updates);

    // Before execute: latest state should have no contract info (history not indexed).
    let pre_state = provider.provider().latest().unwrap();
    assert_eq!(pre_state.nonce(addr).unwrap(), None);

    let mut stage =
        katana_stage::IndexHistory::new(provider.clone(), TaskManager::current().task_spawner());

    // from=0 triggers the bulk path.
    let input = StageExecutionInput::new(0, 3);
    let output = stage.execute(&input).await.expect("execute must succeed");
    assert_eq!(output.last_block_processed, 3);

    // After execute: latest state reflects the indexed changes.
    let latest = provider.provider().latest().unwrap();
    assert_eq!(latest.nonce(addr).unwrap(), Some(nonce));
    assert_eq!(latest.class_hash_of_contract(addr).unwrap(), Some(class_hash));
    assert_eq!(latest.storage(addr, storage_key).unwrap(), Some(storage_value));

    // Historical state at block 1 (when changes happened) should also work.
    let hist = provider.provider().historical(1.into()).unwrap().unwrap();
    assert_eq!(hist.nonce(addr).unwrap(), Some(nonce));
    assert_eq!(hist.class_hash_of_contract(addr).unwrap(), Some(class_hash));
    assert_eq!(hist.storage(addr, storage_key).unwrap(), Some(storage_value));

    // Historical state at block 0 (before changes) should have no data.
    let hist0 = provider.provider().historical(0.into()).unwrap().unwrap();
    assert_eq!(hist0.nonce(addr).unwrap(), None);
    assert_eq!(hist0.class_hash_of_contract(addr).unwrap(), None);
    assert_eq!(hist0.storage(addr, storage_key).unwrap(), None);
}

/// Incremental sync (from>0) uses the per-block write path. Verify it works after
/// a previous execution has already indexed some blocks.
#[tokio::test]
async fn execute_incremental_sync_indexes_state_correctly() {
    let addr = ContractAddress::from(felt!("0x1"));

    let mut updates = BTreeMap::new();
    updates.insert(
        1,
        state_updates_with_contract_changes(
            addr,
            felt!("0xAA"),
            felt!("0x1"),
            felt!("0x10"),
            felt!("0x20"),
        ),
    );
    updates.insert(
        4,
        state_updates_with_contract_changes(
            addr,
            felt!("0xBB"),
            felt!("0x5"),
            felt!("0x10"),
            felt!("0x99"),
        ),
    );

    let provider = create_provider_with_block_data_only(0..=5, updates);

    let mut stage =
        katana_stage::IndexHistory::new(provider.clone(), TaskManager::current().task_spawner());

    // First sync: index blocks 0..=3 (bulk path).
    let output = stage.execute(&StageExecutionInput::new(0, 3)).await.expect("first sync");
    assert_eq!(output.last_block_processed, 3);

    // At this point, block 4's state updates are not yet indexed.
    let latest = provider.provider().latest().unwrap();
    assert_eq!(latest.class_hash_of_contract(addr).unwrap(), Some(felt!("0xAA")));

    // Incremental sync: index blocks 4..=5 (per-block path, from > 0).
    let output = stage.execute(&StageExecutionInput::new(4, 5)).await.expect("incremental sync");
    assert_eq!(output.last_block_processed, 5);

    // Now block 4's updates should be reflected.
    let latest = provider.provider().latest().unwrap();
    assert_eq!(latest.class_hash_of_contract(addr).unwrap(), Some(felt!("0xBB")));
    assert_eq!(latest.nonce(addr).unwrap(), Some(felt!("0x5")));
    assert_eq!(latest.storage(addr, felt!("0x10")).unwrap(), Some(felt!("0x99")));

    // Historical at block 3 should still reflect the old state.
    let hist3 = provider.provider().historical(3.into()).unwrap().unwrap();
    assert_eq!(hist3.class_hash_of_contract(addr).unwrap(), Some(felt!("0xAA")));
    assert_eq!(hist3.nonce(addr).unwrap(), Some(felt!("0x1")));
    assert_eq!(hist3.storage(addr, felt!("0x10")).unwrap(), Some(felt!("0x20")));
}

/// Bulk path correctly handles the same contract being updated across multiple blocks —
/// only the final value should appear in `ContractInfo`/`ContractStorage`, but all
/// intermediate history entries must be present.
#[tokio::test]
async fn execute_bulk_path_accumulates_overlapping_updates() {
    let addr = ContractAddress::from(felt!("0x1"));
    let storage_key = felt!("0x10");

    let mut updates = BTreeMap::new();

    // Block 1: deploy + initial values
    updates.insert(
        1,
        state_updates_with_contract_changes(
            addr,
            felt!("0xAA"),
            felt!("0x1"),
            storage_key,
            felt!("0x100"),
        ),
    );

    // Block 2: update nonce and storage (same contract, same storage key)
    {
        let mut su = StateUpdates::default();
        su.nonce_updates.insert(addr, felt!("0x2"));
        su.storage_updates.insert(addr, BTreeMap::from([(storage_key, felt!("0x200"))]));
        updates.insert(2, StateUpdatesWithClasses { state_updates: su, ..Default::default() });
    }

    // Block 3: replace class and update nonce again
    {
        let mut su = StateUpdates::default();
        su.replaced_classes.insert(addr, felt!("0xBB"));
        su.nonce_updates.insert(addr, felt!("0x3"));
        updates.insert(3, StateUpdatesWithClasses { state_updates: su, ..Default::default() });
    }

    let provider = create_provider_with_block_data_only(0..=3, updates);

    let mut stage =
        katana_stage::IndexHistory::new(provider.clone(), TaskManager::current().task_spawner());

    stage.execute(&StageExecutionInput::new(0, 3)).await.expect("bulk execute");

    // Latest state should reflect the final values from block 3.
    let latest = provider.provider().latest().unwrap();
    assert_eq!(latest.class_hash_of_contract(addr).unwrap(), Some(felt!("0xBB")));
    assert_eq!(latest.nonce(addr).unwrap(), Some(felt!("0x3")));
    assert_eq!(latest.storage(addr, storage_key).unwrap(), Some(felt!("0x200")));

    // Historical at block 1 should show the initial state.
    let hist1 = provider.provider().historical(1.into()).unwrap().unwrap();
    assert_eq!(hist1.class_hash_of_contract(addr).unwrap(), Some(felt!("0xAA")));
    assert_eq!(hist1.nonce(addr).unwrap(), Some(felt!("0x1")));
    assert_eq!(hist1.storage(addr, storage_key).unwrap(), Some(felt!("0x100")));

    // Historical at block 2 should show the intermediate state.
    let hist2 = provider.provider().historical(2.into()).unwrap().unwrap();
    assert_eq!(hist2.class_hash_of_contract(addr).unwrap(), Some(felt!("0xAA")));
    assert_eq!(hist2.nonce(addr).unwrap(), Some(felt!("0x2")));
    assert_eq!(hist2.storage(addr, storage_key).unwrap(), Some(felt!("0x200")));
}

/// Blocks with no state changes should not cause errors on either path.
#[tokio::test]
async fn execute_with_empty_state_updates() {
    // No state updates for any block — all empty.
    let provider = create_provider_with_block_data_only(0..=3, BTreeMap::new());

    let mut stage =
        katana_stage::IndexHistory::new(provider.clone(), TaskManager::current().task_spawner());

    // Bulk path (from=0).
    let output = stage.execute(&StageExecutionInput::new(0, 3)).await.expect("execute empty bulk");
    assert_eq!(output.last_block_processed, 3);

    // Incremental path (from>0) — re-create with block data only for new range.
    let provider2 = create_provider_with_block_data_only(0..=5, BTreeMap::new());
    let mut stage2 =
        katana_stage::IndexHistory::new(provider2.clone(), TaskManager::current().task_spawner());

    // Simulate that blocks 0..=3 were already indexed.
    let output = stage2.execute(&StageExecutionInput::new(4, 5)).await.expect("execute empty incr");
    assert_eq!(output.last_block_processed, 5);
}

/// Prune with no prune range (archive mode) is a no-op.
#[tokio::test]
async fn prune_archive_mode_is_noop() {
    let provider = create_provider_with_block_range(0..=5, BTreeMap::new());
    let mut stage =
        katana_stage::IndexHistory::new(provider.clone(), TaskManager::current().task_spawner());

    // distance=None means archive mode.
    let output = stage.prune(&PruneInput::new(5, None, None)).await.expect("prune");
    assert_eq!(output.pruned_count, 0);
    assert_eq!(provider.provider_mut().earliest_available_state_block().unwrap(), Some(0));
}

// ---- prune tests ----

#[tokio::test]
async fn prune_compacts_state_history_at_boundary() {
    let contract_address = ContractAddress::from(felt!("0x123"));
    let class_hash = felt!("0xCAFE");
    let nonce = felt!("0x7");
    let storage_key = felt!("0x99");
    let storage_value = felt!("0xBEEF");

    // Only block 1 has state updates; later blocks are unchanged.
    let mut updates_by_block = BTreeMap::new();
    updates_by_block.insert(
        1,
        state_updates_with_contract_changes(
            contract_address,
            class_hash,
            nonce,
            storage_key,
            storage_value,
        ),
    );

    let provider = create_provider_with_block_range(0..=8, updates_by_block);
    let mut stage =
        katana_stage::IndexHistory::new(provider.clone(), TaskManager::current().task_spawner());

    // keep_from = 5, prune range = [0, 5)
    let output = stage.prune(&PruneInput::new(8, Some(3), None)).await.expect("prune must succeed");
    assert!(output.pruned_count > 0, "expected prune to delete history entries");
    assert_eq!(provider.provider_mut().earliest_available_state_block().unwrap(), Some(5));

    // Historical reads in the retained window must still work due to anchor compaction.
    let retained_state = provider.provider().historical(7.into()).unwrap().unwrap();
    assert_eq!(retained_state.class_hash_of_contract(contract_address).unwrap(), Some(class_hash));
    assert_eq!(retained_state.nonce(contract_address).unwrap(), Some(nonce));
    assert_eq!(retained_state.storage(contract_address, storage_key).unwrap(), Some(storage_value));

    // Pruned historical state provider range should be unavailable.
    assert!(matches!(
        provider.provider().historical(2.into()),
        Err(ProviderError::HistoricalStatePruned { requested: 2, earliest_available: 5 })
    ));

    // Canonical per-block diffs remain exact even after history compaction.
    assert_eq!(
        provider.provider().state_update(1.into()).unwrap(),
        Some(
            state_updates_with_contract_changes(
                contract_address,
                class_hash,
                nonce,
                storage_key,
                storage_value,
            )
            .state_updates
        )
    );
    assert_eq!(provider.provider().state_update(5.into()).unwrap(), Some(StateUpdates::default()));
}

#[tokio::test]
async fn historical_returns_pruned_error_below_retention_boundary() {
    let provider = create_provider_with_block_range(0..=8, BTreeMap::new());

    let provider_mut = provider.provider_mut();
    provider_mut.set_earliest_available_state_block(5).unwrap();
    provider_mut.commit().unwrap();

    assert!(matches!(
        provider.provider().historical(4.into()),
        Err(ProviderError::HistoricalStatePruned { requested: 4, earliest_available: 5 })
    ));
    assert!(matches!(
        provider.provider().historical(3.into()),
        Err(ProviderError::HistoricalStatePruned { requested: 3, earliest_available: 5 })
    ));
    assert!(provider.provider().historical(5.into()).unwrap().is_some());
    assert!(provider.provider().historical(8.into()).unwrap().is_some());
}

#[tokio::test]
async fn prune_does_not_decrease_existing_retention_boundary() {
    let provider = create_provider_with_block_range(0..=8, BTreeMap::new());
    let mut stage =
        katana_stage::IndexHistory::new(provider.clone(), TaskManager::current().task_spawner());

    let provider_mut = provider.provider_mut();
    provider_mut.set_earliest_available_state_block(10).unwrap();
    provider_mut.commit().unwrap();

    // keep_from=5

    stage.prune(&PruneInput::new(8, Some(3), None)).await.expect("prune must succeed");
    assert_eq!(provider.provider_mut().earliest_available_state_block().unwrap(), Some(10));
}
