use katana_provider::api::block::{BlockNumberProvider, BlockProvider};
use katana_provider::api::env::BlockEnvProvider;
use katana_rpc_server::api::dev::DevApiClient;
use katana_utils::TestNode;

#[tokio::test]
async fn test_next_block_timestamp_in_past() {
    let sequencer = TestNode::new().await;
    let backend = sequencer.backend();
    let provider = backend.storage.provider();

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
    let provider = backend.storage.provider();

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
    let provider = backend.storage.provider();

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

// #[tokio::test]
// async fn test_set_storage_at_on_instant_mode() {
//     let sequencer = create_test_sequencer().await;
//     sequencer.backend().mine_empty_block();

//     let contract_address = ContractAddress(patricia_key!("0x1337"));
//     let key = StorageKey(patricia_key!("0x20"));
//     let val = stark_felt!("0xABC");

//     {
//         let mut state = sequencer.backend().state.write().await;
//         let read_val = state.get_storage_at(contract_address, key).unwrap();
//         assert_eq!(stark_felt!("0x0"), read_val, "latest storage value should be 0");
//     }

//     sequencer.set_storage_at(contract_address, key, val).await.unwrap();

//     {
//         let mut state = sequencer.backend().state.write().await;
//         let read_val = state.get_storage_at(contract_address, key).unwrap();
//         assert_eq!(val, read_val, "latest storage value incorrect after generate");
//     }
// }
