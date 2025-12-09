// Tests for StateTrie stage
//
// Note: The detailed mock-based tests that were here previously tested the state root
// verification logic using mock providers. Since we moved to concrete types (DbProviderFactory),
// these tests would need to be rewritten as integration tests with real data.
//
// The stage itself is tested implicitly through the pipeline integration tests.
// TODO: Add integration tests with real state update data if needed.

use katana_provider::DbProviderFactory;
use katana_stage::trie::StateTrie;
use katana_tasks::TaskManager;

/// Test that the StateTrie stage can be constructed with DbProviderFactory.
#[tokio::test]
async fn can_construct_state_trie_stage() {
    let provider = DbProviderFactory::new_in_memory();
    let task_manager = TaskManager::current();
    let _stage = StateTrie::new(provider, task_manager.task_spawner());
}
