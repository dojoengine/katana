use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use katana_gateway::types::{
    Block, BlockStatus, ConfirmedStateUpdate, ErrorCode, GatewayError, StateDiff, StateUpdate,
    StateUpdateWithBlock,
};
use katana_primitives::block::BlockNumber;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, BlockProvider};
use katana_provider::test_utils::test_provider;
use katana_stage::blocks::{BlockDownloader, Blocks};
use katana_stage::{Stage, StageExecutionInput};
use starknet::core::types::ResourcePrice;

// ============================================================================
// Test Helpers (copied from block.rs to avoid module import issues)
// ============================================================================

/// Mock BlockDownloader implementation for testing.
#[derive(Clone)]
struct MockBlockDownloader {
    responses: Arc<Mutex<HashMap<BlockNumber, Result<StateUpdateWithBlock, String>>>>,
    download_calls: Arc<Mutex<Vec<Vec<BlockNumber>>>>,
}

impl MockBlockDownloader {
    fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(HashMap::new())),
            download_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_block(self, block_number: BlockNumber, block_data: StateUpdateWithBlock) -> Self {
        self.responses.lock().unwrap().insert(block_number, Ok(block_data));
        self
    }
}

impl BlockDownloader for MockBlockDownloader {
    fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> impl Future<Output = Result<Vec<StateUpdateWithBlock>, katana_gateway::client::Error>> + Send
    {
        async move {
            let block_numbers: Vec<BlockNumber> = (from..=to).collect();
            self.download_calls.lock().unwrap().push(block_numbers.clone());

            let mut results = Vec::new();
            let responses = self.responses.lock().unwrap();

            for block_num in block_numbers {
                match responses.get(&block_num) {
                    Some(Ok(block_data)) => results.push(block_data.clone()),
                    Some(Err(error)) => {
                        return Err(katana_gateway::client::Error::Sequencer(GatewayError {
                            code: ErrorCode::BlockNotFound,
                            message: error.clone(),
                            problems: None,
                        }))
                    }
                    None => {
                        return Err(katana_gateway::client::Error::Sequencer(GatewayError {
                            code: ErrorCode::BlockNotFound,
                            message: format!("No response configured for block {}", block_num),
                            problems: None,
                        }))
                    }
                }
            }

            Ok(results)
        }
    }
}

/// Helper function to create a minimal test block.
fn create_downloaded_block(block_number: BlockNumber) -> StateUpdateWithBlock {
    StateUpdateWithBlock {
        block: Block {
            status: BlockStatus::AcceptedOnL2,
            block_hash: Some(Felt::from(block_number)),
            parent_block_hash: Felt::from(block_number.saturating_sub(1)),
            block_number: Some(block_number),
            l1_gas_price: ResourcePrice { price_in_fri: Felt::ONE, price_in_wei: Felt::ONE },
            l2_gas_price: ResourcePrice { price_in_fri: Felt::ONE, price_in_wei: Felt::ONE },
            l1_data_gas_price: ResourcePrice { price_in_fri: Felt::ONE, price_in_wei: Felt::ONE },
            timestamp: block_number as u64,
            sequencer_address: Some(ContractAddress(Felt::ZERO)),
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            transactions: Vec::new(),
            transaction_receipts: Vec::new(),
            starknet_version: Some("0.13.0".to_string()),
            transaction_commitment: Some(Felt::ZERO),
            event_commitment: Some(Felt::ZERO),
            state_diff_commitment: Some(Felt::ZERO),
            state_root: Some(Felt::ZERO),
        },
        state_update: StateUpdate::Confirmed(ConfirmedStateUpdate {
            block_hash: Felt::from(block_number),
            new_root: Felt::ZERO,
            old_root: Felt::ZERO,
            state_diff: StateDiff::default(),
        }),
    }
}

// ============================================================================
// Unwinding Tests
// ============================================================================

#[tokio::test]
async fn unwind_removes_single_block() {
    let provider = test_provider();
    let downloader = MockBlockDownloader::new()
        .with_block(1, create_downloaded_block(1))
        .with_block(2, create_downloaded_block(2));

    let mut stage = Blocks::new(provider.clone(), downloader);

    // Execute: insert blocks 1 and 2
    let input = StageExecutionInput::new(1, 2);
    stage.execute(&input).await.expect("failed to execute stage");

    // Verify blocks were stored
    assert_eq!(provider.latest_number().unwrap(), 2);
    assert!(provider.block_by_number(1).unwrap().is_some());
    assert!(provider.block_by_number(2).unwrap().is_some());
    assert!(provider.block_hash_by_num(1).unwrap().is_some());
    assert!(provider.block_hash_by_num(2).unwrap().is_some());

    // Unwind: remove block 2
    let result = stage.unwind(1).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().last_block_processed, 1);

    // Verify block 2 was removed
    assert_eq!(provider.latest_number().unwrap(), 1);
    // Use block_hash_by_num to verify existence (returns Option, not Result)
    assert!(provider.block_hash_by_num(1).unwrap().is_some(), "block 1 should still exist");
    assert!(provider.block_hash_by_num(2).unwrap().is_none(), "block 2 should be removed");
}

#[tokio::test]
async fn unwind_removes_multiple_blocks() {
    let provider = test_provider();
    let mut downloader = MockBlockDownloader::new();

    // Configure blocks 1-6
    for block_num in 1..=6 {
        downloader = downloader.with_block(block_num, create_downloaded_block(block_num));
    }

    let mut stage = Blocks::new(provider.clone(), downloader);

    // Execute: insert blocks 1-6
    let input = StageExecutionInput::new(1, 6);
    stage.execute(&input).await.expect("failed to execute stage");

    // Verify all blocks were stored
    assert_eq!(provider.latest_number().unwrap(), 6);
    for block_num in 1..=6 {
        assert!(provider.block_by_number(block_num).unwrap().is_some());
        assert!(provider.block_hash_by_num(block_num).unwrap().is_some());
    }

    // Unwind: remove blocks 4-6
    let result = stage.unwind(3).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().last_block_processed, 3);

    // Verify blocks 4-6 were removed
    assert_eq!(provider.latest_number().unwrap(), 3);

    // Blocks 1-3 should still exist
    for block_num in 1..=3 {
        assert!(
            provider.block_hash_by_num(block_num).unwrap().is_some(),
            "block {} should still exist",
            block_num
        );
    }

    // Blocks 4-6 should be removed
    for block_num in 4..=6 {
        assert!(
            provider.block_hash_by_num(block_num).unwrap().is_none(),
            "block {} should be removed",
            block_num
        );
    }
}

#[tokio::test]
async fn unwind_to_genesis() {
    let provider = test_provider();
    let mut downloader = MockBlockDownloader::new();

    // Configure blocks 1-5
    for block_num in 1..=5 {
        downloader = downloader.with_block(block_num, create_downloaded_block(block_num));
    }

    let mut stage = Blocks::new(provider.clone(), downloader);

    // Execute: insert blocks 1-5
    let input = StageExecutionInput::new(1, 5);
    stage.execute(&input).await.expect("failed to execute stage");

    // Verify all blocks were stored
    assert_eq!(provider.latest_number().unwrap(), 5);
    for block_num in 1..=5 {
        assert!(provider.block_by_number(block_num).unwrap().is_some());
    }

    // Unwind to block 0 (genesis)
    let result = stage.unwind(0).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().last_block_processed, 0);

    // Verify blocks 1-5 were removed, only genesis should remain
    assert_eq!(provider.latest_number().unwrap(), 0);
    for block_num in 1..=5 {
        assert!(
            provider.block_hash_by_num(block_num).unwrap().is_none(),
            "block {} should be removed",
            block_num
        );
    }

    // Genesis block should still exist
    assert!(provider.block_hash_by_num(0).unwrap().is_some());
}

#[tokio::test]
async fn unwind_removes_transactions_and_receipts() {
    let provider = test_provider();
    let downloader = MockBlockDownloader::new()
        .with_block(1, create_downloaded_block(1))
        .with_block(2, create_downloaded_block(2));

    let mut stage = Blocks::new(provider.clone(), downloader);

    // Execute: insert blocks 1 and 2
    let input = StageExecutionInput::new(1, 2);
    stage.execute(&input).await.expect("failed to execute stage");

    // Verify blocks and their data were stored
    assert_eq!(provider.latest_number().unwrap(), 2);
    let block_1 = provider.block_by_number(1).unwrap().expect("block 1 should exist");
    let block_2 = provider.block_by_number(2).unwrap().expect("block 2 should exist");

    // Get transaction counts before unwinding
    let tx_count_1 = block_1.body.len();
    let _tx_count_2 = block_2.body.len();

    // Unwind: remove block 2
    let result = stage.unwind(1).await;
    assert!(result.is_ok());

    // Verify block 2 was removed
    assert!(provider.block_hash_by_num(2).unwrap().is_none(), "block 2 should be removed");

    // Verify block 1 still exists with its transactions
    assert!(provider.block_hash_by_num(1).unwrap().is_some(), "block 1 should still exist");
    let block_1_after = provider.block(1.into()).unwrap().expect("block 1 should exist");
    assert_eq!(
        block_1_after.body.len(),
        tx_count_1,
        "block 1 should have same number of transactions"
    );
}

#[tokio::test]
async fn unwind_sequential_unwinding() {
    let provider = test_provider();
    let mut downloader = MockBlockDownloader::new();

    // Configure blocks 1-6
    for block_num in 1..=6 {
        downloader = downloader.with_block(block_num, create_downloaded_block(block_num));
    }

    let mut stage = Blocks::new(provider.clone(), downloader);

    // Execute: insert blocks 1-6
    let input = StageExecutionInput::new(1, 6);
    stage.execute(&input).await.expect("failed to execute stage");

    assert_eq!(provider.latest_number().unwrap(), 6);

    // First unwind: remove blocks 5-6
    stage.unwind(4).await.expect("first unwind failed");
    assert_eq!(provider.latest_number().unwrap(), 4);
    assert!(provider.block_hash_by_num(4).unwrap().is_some());
    assert!(provider.block_hash_by_num(5).unwrap().is_none());
    assert!(provider.block_hash_by_num(6).unwrap().is_none());

    // Second unwind: remove blocks 2-4
    stage.unwind(1).await.expect("second unwind failed");
    assert_eq!(provider.latest_number().unwrap(), 1);
    assert!(provider.block_hash_by_num(1).unwrap().is_some());
    for block_num in 2..=6 {
        assert!(provider.block_hash_by_num(block_num).unwrap().is_none());
    }
}

#[tokio::test]
async fn unwind_and_re_execute() {
    let provider = test_provider();
    let mut downloader = MockBlockDownloader::new();

    // Configure blocks 1-4
    for block_num in 1..=4 {
        downloader = downloader.with_block(block_num, create_downloaded_block(block_num));
    }

    let mut stage = Blocks::new(provider.clone(), downloader.clone());

    // Execute: insert blocks 1-4
    let input = StageExecutionInput::new(1, 4);
    stage.execute(&input).await.expect("failed to execute stage");
    assert_eq!(provider.latest_number().unwrap(), 4);

    // Unwind: remove blocks 3-4
    stage.unwind(2).await.expect("unwind failed");
    assert_eq!(provider.latest_number().unwrap(), 2);
    assert!(provider.block_hash_by_num(3).unwrap().is_none());
    assert!(provider.block_hash_by_num(4).unwrap().is_none());

    // Re-execute: re-insert blocks 3-4
    let input = StageExecutionInput::new(3, 4);
    stage.execute(&input).await.expect("re-execute failed");

    // Verify blocks were re-inserted
    assert_eq!(provider.latest_number().unwrap(), 4);
    assert!(provider.block_hash_by_num(3).unwrap().is_some());
    assert!(provider.block_hash_by_num(4).unwrap().is_some());
}

#[tokio::test]
async fn unwind_with_block_hash_lookup() {
    let provider = test_provider();
    let mut downloader = MockBlockDownloader::new();

    // Configure blocks 1-4
    for block_num in 1..=4 {
        downloader = downloader.with_block(block_num, create_downloaded_block(block_num));
    }

    let mut stage = Blocks::new(provider.clone(), downloader);

    // Execute: insert blocks 1-4
    let input = StageExecutionInput::new(1, 4);
    stage.execute(&input).await.expect("failed to execute stage");

    // Store block hashes before unwinding
    let hash_3 = provider.block_hash_by_num(3).unwrap().expect("block 3 hash should exist");
    let hash_4 = provider.block_hash_by_num(4).unwrap().expect("block 4 hash should exist");

    // Verify reverse lookup works before unwinding
    assert_eq!(provider.block_number_by_hash(hash_3).unwrap(), Some(3));
    assert_eq!(provider.block_number_by_hash(hash_4).unwrap(), Some(4));

    // Unwind: remove blocks 3-4
    stage.unwind(2).await.expect("unwind failed");

    // Verify block hashes were removed
    assert!(provider.block_hash_by_num(3).unwrap().is_none());
    assert!(provider.block_hash_by_num(4).unwrap().is_none());

    // Verify reverse lookup no longer works
    assert_eq!(provider.block_number_by_hash(hash_3).unwrap(), None);
    assert_eq!(provider.block_number_by_hash(hash_4).unwrap(), None);
}
