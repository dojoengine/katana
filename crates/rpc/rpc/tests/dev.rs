use assert_matches::assert_matches;
use cainome::rs::abigen_legacy;
use katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::contract::ContractAddress;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockNumberProvider, BlockProvider};
use katana_provider::api::env::BlockEnvProvider;
use katana_provider::api::state::StateFactoryProvider;
use katana_rpc::api::dev::DevApiClient;
use katana_rpc_client::starknet::{Error as RpcError, StarknetApiError};
use katana_utils::TestNode;

abigen_legacy!(Erc20Contract, "crates/contracts/build/legacy/erc20.json");

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
    let provider = sequencer.starknet_rpc_client();

    // Use the fee token address which is always deployed
    let contract_address: ContractAddress = DEFAULT_ETH_FEE_TOKEN_ADDRESS.into();
    // Use a storage key that is unlikely to be used by the contract
    let key = Felt::from(0x1337u64);
    let value = Felt::from(0xABCu64);

    // Check that storage is initially None/zero
    {
        let db_provider = backend.blockchain.provider();
        let state = db_provider.latest().unwrap();
        let read_val = state.storage(contract_address, key).unwrap();
        assert!(read_val.is_none(), "initial storage value should be None");
    }

    // Set the storage value via RPC
    client.set_storage_at(contract_address, key, value).await.unwrap();

    // Verify the storage value was set correctly via direct provider access
    {
        let db_provider = backend.blockchain.provider();
        let state = db_provider.latest().unwrap();
        let read_val = state.storage(contract_address, key).unwrap();
        assert_eq!(read_val, Some(value), "storage value should be set correctly");
    }

    // Verify the storage value via starknet_getStorageAt RPC method
    let rpc_value =
        provider.get_storage_at(contract_address.into(), key, BlockIdOrTag::Latest).await.unwrap();
    assert_eq!(rpc_value, value, "storage value should be accessible via RPC");
}

/// Test set_storage_at in no mining mode (with pending block)
/// This verifies that the storage update is visible in the pending state and persists after mining.
#[tokio::test]
async fn test_set_storage_at_with_pending_block() {
    // Create a node with no mining mode - blocks are only created via dev_generateBlock
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = true;
    let sequencer = TestNode::new_with_config(config).await;

    let backend = sequencer.backend();
    let client = sequencer.rpc_http_client();
    let provider = sequencer.starknet_rpc_client();
    let account = sequencer.account();

    // Use the fee token address which is always deployed
    let contract_address: ContractAddress = DEFAULT_ETH_FEE_TOKEN_ADDRESS.into();
    // Use a storage key that is unlikely to be used by the contract
    let key = Felt::from(0x1337u64);
    let value = Felt::from(0xABCu64);

    // Send a transaction to trigger the creation of a pending block.
    // In no mining mode, the pending block opens after receiving the first transaction.
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };
    let res = contract.transfer(&Felt::ONE, &amount).send().await.unwrap();
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();

    // Verify storage is initially zero before setting it
    let initial_value = provider
        .get_storage_at(contract_address.into(), key, BlockIdOrTag::PreConfirmed)
        .await
        .unwrap();
    assert_eq!(initial_value, Felt::ZERO, "storage should be zero before setting");

    // Set the storage value via RPC - this updates the pending state
    client.set_storage_at(contract_address, key, value).await.unwrap();

    // Verify the storage value is visible in the pending state (before mining)
    let pending_value = provider
        .get_storage_at(contract_address.into(), key, BlockIdOrTag::PreConfirmed)
        .await
        .unwrap();
    assert_eq!(pending_value, value, "storage value should be visible in pending state");

    // Force mine a block to close the pending block and persist the changes
    client.generate_block().await.unwrap();

    // Verify the storage value was persisted to the database after the block was mined
    {
        let db_provider = backend.blockchain.provider();
        let state = db_provider.latest().unwrap();
        let read_val = state.storage(contract_address, key).unwrap();
        assert_eq!(read_val, Some(value), "storage value should persist after block is mined");
    }

    // Verify the storage value via starknet_getStorageAt RPC method
    let rpc_value =
        provider.get_storage_at(contract_address.into(), key, BlockIdOrTag::Latest).await.unwrap();
    assert_eq!(rpc_value, value, "storage value should be accessible via RPC");
}

/// Test set_storage_at for a contract that doesn't exist.
/// The storage should be set successfully, but starknet_getStorageAt should return
/// ContractNotFound.
#[tokio::test]
async fn test_set_storage_at_nonexistent_contract() {
    let sequencer = TestNode::new().await;
    let backend = sequencer.backend();
    let client = sequencer.rpc_http_client();
    let provider = sequencer.starknet_rpc_client();

    // Use an address that doesn't have a contract deployed
    let contract_address = ContractAddress(Felt::from(0x1337u64));
    let key = Felt::from(0x20u64);
    let value = Felt::from(0xABCu64);

    // Set the storage value via RPC - this should succeed even for non-existent contracts
    client.set_storage_at(contract_address, key, value).await.unwrap();

    // Verify the storage value was set correctly via direct provider access
    {
        let db_provider = backend.blockchain.provider();
        let state = db_provider.latest().unwrap();
        let read_val = state.storage(contract_address, key).unwrap();
        assert_eq!(read_val, Some(value), "storage value should be set in database");
    }

    // Verify that starknet_getStorageAt returns ContractNotFound for non-existent contract
    let result = provider.get_storage_at(contract_address.into(), key, BlockIdOrTag::Latest).await;

    assert_matches!(
        result,
        Err(RpcError::Starknet(StarknetApiError::ContractNotFound)),
        "starknet_getStorageAt should return ContractNotFound for non-existent contract"
    );
}
