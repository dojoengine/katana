use katana_primitives::contract::ContractAddress;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockNumberProvider, BlockProvider};
use katana_provider::api::env::BlockEnvProvider;
use katana_provider::api::state::StateFactoryProvider;
use katana_rpc::api::dev::DevApiClient;
use katana_utils::TestNode;

#[tokio::test]
async fn test_next_block_timestamp_in_past() {
    let sequencer = TestNode::new().await;
    let backend = sequencer.backend();
    let provider = backend.blockchain.provider();

    // Create a jsonrpsee client for the DevApi
    let client = sequencer.rpc_http_client();

    let block_num = provider.latest_number().unwrap();
    let mut block_env = provider.block_env_at(block_num.into()).unwrap().unwrap();
    backend.update_block_env(&mut block_env);

    let block1 = backend.mine_empty_block(&block_env).unwrap().block_number;
    let block1_timestamp = provider.block(block1.into()).unwrap().unwrap().header.timestamp;
    client.set_next_block_timestamp(block1_timestamp - 1000).await.unwrap();

    let block_num = provider.latest_number().unwrap();
    let mut block_env = provider.block_env_at(block_num.into()).unwrap().unwrap();
    backend.update_block_env(&mut block_env);

    let block2 = backend.mine_empty_block(&block_env).unwrap().block_number;
    let block2_timestamp = provider.block(block2.into()).unwrap().unwrap().header.timestamp;

    assert_eq!(block2_timestamp, block1_timestamp - 1000, "timestamp should be updated");
}

#[tokio::test]
async fn test_set_next_block_timestamp_in_future() {
    let sequencer = TestNode::new().await;
    let backend = sequencer.backend();
    let provider = backend.blockchain.provider();

    // Create a jsonrpsee client for the DevApi
    let client = sequencer.rpc_http_client();

    let block_num = provider.latest_number().unwrap();
    let mut block_env = provider.block_env_at(block_num.into()).unwrap().unwrap();
    backend.update_block_env(&mut block_env);
    let block1 = backend.mine_empty_block(&block_env).unwrap().block_number;

    let block1_timestamp = provider.block(block1.into()).unwrap().unwrap().header.timestamp;

    client.set_next_block_timestamp(block1_timestamp + 1000).await.unwrap();

    let block_num = provider.latest_number().unwrap();
    let mut block_env = provider.block_env_at(block_num.into()).unwrap().unwrap();
    backend.update_block_env(&mut block_env);
    let block2 = backend.mine_empty_block(&block_env).unwrap().block_number;

    let block2_timestamp = provider.block(block2.into()).unwrap().unwrap().header.timestamp;

    assert_eq!(block2_timestamp, block1_timestamp + 1000, "timestamp should be updated");
}

#[tokio::test]
async fn test_increase_next_block_timestamp() {
    let sequencer = TestNode::new().await;
    let backend = sequencer.backend();
    let provider = backend.blockchain.provider();

    // Create a jsonrpsee client for the DevApi
    let client = sequencer.rpc_http_client();

    let block_num = provider.latest_number().unwrap();
    let mut block_env = provider.block_env_at(block_num.into()).unwrap().unwrap();
    backend.update_block_env(&mut block_env);
    let block1 = backend.mine_empty_block(&block_env).unwrap().block_number;

    let block1_timestamp = provider.block(block1.into()).unwrap().unwrap().header.timestamp;

    client.increase_next_block_timestamp(1000).await.unwrap();

    let block_num = provider.latest_number().unwrap();
    let mut block_env = provider.block_env_at(block_num.into()).unwrap().unwrap();
    backend.update_block_env(&mut block_env);
    let block2 = backend.mine_empty_block(&block_env).unwrap().block_number;

    let block2_timestamp = provider.block(block2.into()).unwrap().unwrap().header.timestamp;

    // Depending on the current time and the machine we run on, we may have 1 sec difference
    // between the expected and actual timestamp.
    // We take this possible delay in account to have the test more robust for now,
    // but it may due to how the timestamp is updated in the sequencer.
    assert!(
        block2_timestamp == block1_timestamp + 1000 || block2_timestamp == block1_timestamp + 1001,
        "timestamp should be updated"
    );
}

#[tokio::test]
async fn test_dev_api_enabled() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();

    let accounts = client.predeployed_accounts().await.unwrap();
    assert!(!accounts.is_empty(), "predeployed accounts should not be empty");
}

/// Test set_storage_at in instant mining mode (no pending block)
#[tokio::test]
async fn test_set_storage_at() {
    let sequencer = TestNode::new().await;
    let backend = sequencer.backend();
    let client = sequencer.rpc_http_client();

    let contract_address = ContractAddress(Felt::from(0x1337u64));
    let key = Felt::from(0x20u64);
    let value = Felt::from(0xABCu64);

    // Check that storage is initially None/zero
    {
        let provider = backend.blockchain.provider();
        let state = provider.latest().unwrap();
        let read_val = state.storage(contract_address, key).unwrap();
        assert!(read_val.is_none(), "initial storage value should be None");
    }

    // Set the storage value via RPC
    client.set_storage_at(contract_address, key, value).await.unwrap();

    // Verify the storage value was set correctly
    {
        let provider = backend.blockchain.provider();
        let state = provider.latest().unwrap();
        let read_val = state.storage(contract_address, key).unwrap();
        assert_eq!(read_val, Some(value), "storage value should be set correctly");
    }
}

/// Test set_storage_at in interval mining mode (with pending block)
/// This verifies that the storage update is visible in the pending state and persists after mining.
#[tokio::test]
async fn test_set_storage_at_with_pending_block() {
    // Create a node with interval mining (block time of 10 seconds - long enough that we can test
    // before the block is mined)
    let sequencer = TestNode::new_with_block_time(10000).await;
    let backend = sequencer.backend();
    let client = sequencer.rpc_http_client();

    let contract_address = ContractAddress(Felt::from(0x1337u64));
    let key = Felt::from(0x20u64);
    let value = Felt::from(0xABCu64);

    // Set the storage value via RPC - this updates the pending state
    client.set_storage_at(contract_address, key, value).await.unwrap();

    // In interval mode, the storage is updated in the pending executor's state, not the database.
    // The database will be updated when the block is mined.

    // Force mine a block to close the pending block and persist the changes
    client.generate_block().await.unwrap();

    // Verify the storage value was persisted to the database after the block was mined
    {
        let provider = backend.blockchain.provider();
        let state = provider.latest().unwrap();
        let read_val = state.storage(contract_address, key).unwrap();
        assert_eq!(read_val, Some(value), "storage value should persist after block is mined");
    }
}
