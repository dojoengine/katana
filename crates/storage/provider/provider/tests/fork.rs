use katana_primitives::block::BlockHashOrNumber;
use katana_primitives::felt;
use katana_primitives::transaction::TxType;
use katana_provider::api::block::{BlockHashProvider, BlockProvider};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider::providers::fork::ForkedProvider;
use katana_rpc_client::starknet::Client as StarknetClient;

const SEPOLIA_RPC_URL: &str = "https://api.cartridge.gg/x/starknet/sepolia";
const FORK_BLOCK_NUMBER: u64 = 2888618;

/// Test that the ForkedProvider can fetch block data from the forked network.
///
/// This test validates that when we request a block that exists on the forked network
/// (before the fork point), the provider successfully fetches it from the remote network.
#[tokio::test]
async fn block_from_forked_network() {
    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider = ForkedProvider::new_ephemeral(FORK_BLOCK_NUMBER, starknet_client);

    // Request a block that should exist on the forked network (before the fork point)
    // Using a block number that is well before the fork point
    let block_num = 2888610;
    let block_id = BlockHashOrNumber::Num(block_num);

    let result = provider.block(block_id).unwrap();
    let block = result.expect("block should exist");

    let expected_parent_hash =
        felt!("0x4abdd03c515d0513bc41d923987fd8cf977d9849252bb21777d90542ce7d8");
    let expected_sequencer_address =
        felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8");
    let expected_state_root =
        felt!("0x2a366576ae1fc1a028bdc1a669c4c0da01e858671576fd1fe92340f64990300");
    let expected_timestamp = 1763171340;
    let expected_tx_count = 13;

    assert_eq!(block.header.number, block_num);
    assert_eq!(block.header.parent_hash, expected_parent_hash);
    assert_eq!(block.header.sequencer_address, expected_sequencer_address);
    assert_eq!(block.header.state_root, expected_state_root);
    assert_eq!(block.header.timestamp, expected_timestamp);
    assert_eq!(block.body.len(), expected_tx_count);

    // assert that all related data is populated and can be fetched correctly

    let transactions = provider.transactions_by_block(block_id).unwrap();
    let transactions = transactions.expect("block transactions must be stored");
    assert_eq!(transactions.len(), expected_tx_count);

    let receipts = provider.receipts_by_block(block_id).unwrap();
    let receipts = receipts.expect("block transactions must be stored");
    assert_eq!(receipts.len(), expected_tx_count);

    let state_updates = provider.state_update(block_id).unwrap();
    assert!(state_updates.is_some());
}

#[tokio::test]
async fn block_hash_from_forked_network() {
    let expected_hash = felt!("0x4f3db32fa485be6e8ed6ac7ce715a8739e9a28d67ea575c502e25036b5f178a");

    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider = ForkedProvider::new_ephemeral(FORK_BLOCK_NUMBER, starknet_client);

    let block_num = 2888611;
    let result = provider.block_hash_by_num(block_num).unwrap();
    let block_hash = result.expect("block hash should exist");

    assert_eq!(block_hash, expected_hash);
}

#[tokio::test]
async fn block_after_fork_point_returns_none() {
    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());

    let provider = ForkedProvider::new_ephemeral(FORK_BLOCK_NUMBER, starknet_client);

    // Request a block after the fork point (should not exist locally)
    // The block might exist on the forked network, but since it's after the fork point,
    // the provider should not fetch it and should return None.
    let block_num = FORK_BLOCK_NUMBER + 10;
    let block_id = BlockHashOrNumber::Num(block_num);

    let block = provider.block(block_id).unwrap();
    assert!(block.is_none(), "Block after fork point should return None");
}

#[tokio::test]
async fn transaction_from_forked_network() {
    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider = ForkedProvider::new_ephemeral(FORK_BLOCK_NUMBER, starknet_client);

    let block_id = BlockHashOrNumber::Num(2888610);
    let tx_hash = felt!("0x40042d86e1b52896f3c695b713f3114ca53905890df0e14d09b4c1d51e2b1b0");

    let result = provider.transaction_by_hash(tx_hash).unwrap();
    let tx = result.expect("tx should exist");

    assert_eq!(tx.r#type(), TxType::Invoke);
    // TODO: assert individual fields of the transaction

    // the related block should be fetched too.
    // assert that all related data is populated and can be fetched correctly.

    let forked_db = provider.forked_db(); // bypass the ForkedProvider

    let result = forked_db.block(block_id).unwrap();
    let block = result.expect("block should be populated");

    let expected_parent_hash =
        felt!("0x4abdd03c515d0513bc41d923987fd8cf977d9849252bb21777d90542ce7d8");
    let expected_sequencer_address =
        felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8");
    let expected_state_root =
        felt!("0x2a366576ae1fc1a028bdc1a669c4c0da01e858671576fd1fe92340f64990300");
    let expected_timestamp = 1763171340;
    let expected_tx_count = 13;

    assert_eq!(block.header.number, 2888610);
    assert_eq!(block.header.parent_hash, expected_parent_hash);
    assert_eq!(block.header.sequencer_address, expected_sequencer_address);
    assert_eq!(block.header.state_root, expected_state_root);
    assert_eq!(block.header.timestamp, expected_timestamp);
    assert_eq!(block.body.len(), expected_tx_count);

    let transactions = forked_db.transactions_by_block(block_id).unwrap();
    let transactions = transactions.expect("block transactions must be stored");
    assert_eq!(transactions.len(), expected_tx_count);

    let receipts = forked_db.receipts_by_block(block_id).unwrap();
    let receipts = receipts.expect("block transactions must be stored");
    assert_eq!(receipts.len(), expected_tx_count);

    let state_updates = forked_db.state_update(block_id).unwrap();
    assert!(state_updates.is_some());
}
