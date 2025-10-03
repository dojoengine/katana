use katana_gateway::client::Client;
use katana_gateway::types::{Block, BlockId, StateUpdate, StateUpdateWithBlock};
use katana_primitives::block::BlockNumber;
use rstest::rstest;

mod fixtures;

use fixtures::{gateway, test_data};

#[rstest]
#[case::pre_v0_7_0(0, test_data("pre_0.7.0/block/mainnet_genesis.json"))]
#[case::v0_7_0(2240, test_data("0.7.0/block/mainnet_2240.json"))]
#[case::v0_8_0(2500, test_data("0.8.0/block/mainnet_2500.json"))]
#[case::v0_9_0(2800, test_data("0.9.0/block/mainnet_2800.json"))]
#[case::v0_10_0(6500, test_data("0.10.0/block/mainnet_6500.json"))]
#[case::v0_11_1(65000, test_data("0.11.1/block/mainnet_65000.json"))]
#[case::v0_13_0(550000, test_data("0.13.0/block/mainnet_550000.json"))]
#[case::v0_14_0(2238855, test_data("0.14.0/block/mainnet_2238855.json"))]
#[tokio::test]
async fn get_block(gateway: Client, #[case] block_number: BlockNumber, #[case] expected: Block) {
    let id = BlockId::Number(block_number);
    let block = gateway.get_block(id).await.unwrap();
    similar_asserts::assert_eq!(block, expected);
}

#[rstest]
#[case::pre_v0_7_0(0, test_data("pre_0.7.0/state_update/mainnet_genesis.json"))]
#[case::v0_7_0(2240, test_data("0.7.0/state_update/mainnet_2240.json"))]
#[case::v0_8_0(2500, test_data("0.8.0/state_update/mainnet_2500.json"))]
#[case::v0_9_0(2800, test_data("0.9.0/state_update/mainnet_2800.json"))]
#[case::v0_10_0(6500, test_data("0.10.0/state_update/mainnet_6500.json"))]
#[case::v0_11_1(65000, test_data("0.11.1/state_update/mainnet_65000.json"))]
#[case::v0_12_2(350000, test_data("0.12.2/state_update/mainnet_350000.json"))]
#[case::v0_13_0(550000, test_data("0.13.0/state_update/mainnet_550000.json"))]
#[tokio::test]
async fn get_state_update(
    gateway: Client,
    #[case] block_number: BlockNumber,
    #[case] expected: StateUpdate,
) {
    let id = BlockId::Number(block_number);
    let state_update = gateway.get_state_update(id).await.unwrap();
    similar_asserts::assert_eq!(state_update, expected);
}

#[rstest]
#[case::pre_v0_7_0(0, test_data("pre_0.7.0/state_update_with_block/mainnet_genesis.json"))]
#[case::v0_7_0(2240, test_data("0.7.0/state_update_with_block/mainnet_2240.json"))]
#[case::v0_8_0(2500, test_data("0.8.0/state_update_with_block/mainnet_2500.json"))]
#[case::v0_9_0(2800, test_data("0.9.0/state_update_with_block/mainnet_2800.json"))]
#[case::v0_10_0(6500, test_data("0.10.0/state_update_with_block/mainnet_6500.json"))]
#[case::v0_11_1(65000, test_data("0.11.1/state_update_with_block/mainnet_65000.json"))]
#[case::v0_12_2(350000, test_data("0.12.2/state_update_with_block/mainnet_350000.json"))]
#[case::v0_13_0(550000, test_data("0.13.0/state_update_with_block/mainnet_550000.json"))]
#[tokio::test]
async fn get_state_update_with_block(
    gateway: Client,
    #[case] block_number: BlockNumber,
    #[case] expected: StateUpdateWithBlock,
) {
    let id = BlockId::Number(block_number);
    let state_update = gateway.get_state_update_with_block(id).await.unwrap();
    similar_asserts::assert_eq!(state_update, expected);
}
