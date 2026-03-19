mod common;

use std::collections::BTreeMap;

use common::{create_provider_with_block_range, state_updates_with_contract_changes};
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::{felt, ContractAddress};
use katana_provider::api::state::{
    HistoricalStateRetentionProvider, StateFactoryProvider, StateProvider,
};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::{MutableProvider, ProviderError, ProviderFactory};
use katana_stage::{PruneInput, Stage};
use katana_tasks::TaskManager;

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
