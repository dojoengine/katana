use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use katana_gateway::client::Client as SequencerGateway;
use katana_gateway::types::{
    Block, BlockStatus, ConfirmedStateUpdate, ErrorCode, GatewayError, StateDiff, StateUpdate,
    StateUpdateWithBlock,
};
use katana_primitives::block::{BlockNumber, SealedBlockWithStatus};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::Receipt;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::block::{BlockNumberProvider, BlockWriter};
use katana_provider::test_utils::test_provider;
use katana_provider::ProviderResult;
use katana_stage::blocks::{BatchBlockDownloader, BlockDownloader, Blocks};
use katana_stage::{Stage, StageExecutionInput};
use rstest::rstest;
use starknet::core::types::ResourcePrice;

/// Mock BlockDownloader implementation for testing.
///
/// Allows precise control over download behavior by pre-configuring responses
/// for specific block number ranges or individual blocks.
#[derive(Clone)]
struct MockBlockDownloader {
    /// Map of block number to result (Ok or Err).
    responses: Arc<Mutex<HashMap<BlockNumber, Result<StateUpdateWithBlock, String>>>>,
    /// Track download calls for verification.
    ///
    /// This is used to verify the input of [`BlockDownloader::download_blocks`] .
    download_calls: Arc<Mutex<Vec<Vec<BlockNumber>>>>,
}

impl MockBlockDownloader {
    fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(HashMap::new())),
            download_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Configure a successful response for a specific block number.
    ///
    /// When a block is downloaded via [`BlockDownloader::download_blocks`], the corresponding
    /// `block_data` is returned.
    fn with_block(self, block_number: BlockNumber, block_data: StateUpdateWithBlock) -> Self {
        self.responses.lock().unwrap().insert(block_number, Ok(block_data));
        self
    }

    /// Configure an error response for a specific block number.
    fn with_error(self, block_number: BlockNumber, error: String) -> Self {
        self.responses.lock().unwrap().insert(block_number, Err(error));
        self
    }

    /// Get the number of times download_blocks was called.
    fn download_call_count(&self) -> usize {
        self.download_calls.lock().unwrap().len()
    }

    /// Get all block numbers that were requested across all download calls.
    fn requested_blocks(&self) -> Vec<BlockNumber> {
        self.download_calls
            .lock()
            .unwrap()
            .iter()
            .flat_map(|blocks| blocks.iter().copied())
            .collect()
    }
}

// We're only testing the stage business logic so we don't really care about using the
// BatchDownloader/Downloader combination.
impl BlockDownloader for MockBlockDownloader {
    fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> impl Future<Output = Result<Vec<StateUpdateWithBlock>, katana_gateway::client::Error>> + Send
    {
        async move {
            let block_numbers: Vec<BlockNumber> = (from..=to).collect();

            // Track the call
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
                        }))
                    }
                    None => {
                        return Err(katana_gateway::client::Error::Sequencer(GatewayError {
                            code: ErrorCode::BlockNotFound,
                            message: format!("No response configured for block {}", block_num),
                        }))
                    }
                }
            }

            Ok(results)
        }
    }
}

/// Mock BlockWriter implementation for testing.
///
/// Tracks all insert operations and can be configured to return errors.
#[derive(Clone)]
struct MockBlockWriter {
    /// Stored blocks with their receipts and state updates.
    blocks: Arc<Mutex<Vec<(SealedBlockWithStatus, StateUpdatesWithClasses, Vec<Receipt>)>>>,
    /// Whether to return an error on insert.
    should_fail: Arc<Mutex<bool>>,
    /// Error message to return when should_fail is true.
    error_message: Arc<Mutex<String>>,
}

impl MockBlockWriter {
    fn new() -> Self {
        Self {
            blocks: Arc::new(Mutex::new(Vec::new())),
            should_fail: Arc::new(Mutex::new(false)),
            error_message: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Configure the mock to fail on insert operations.
    fn with_insert_error(self, error: String) -> Self {
        *self.should_fail.lock().unwrap() = true;
        *self.error_message.lock().unwrap() = error;
        self
    }

    /// Get the number of blocks stored.
    fn stored_block_count(&self) -> usize {
        self.blocks.lock().unwrap().len()
    }

    /// Get all stored block numbers.
    fn stored_block_numbers(&self) -> Vec<BlockNumber> {
        self.blocks.lock().unwrap().iter().map(|(block, _, _)| block.block.header.number).collect()
    }
}

impl BlockWriter for MockBlockWriter {
    fn insert_block_with_states_and_receipts(
        &self,
        block: SealedBlockWithStatus,
        states: StateUpdatesWithClasses,
        receipts: Vec<Receipt>,
        _executions: Vec<TypedTransactionExecutionInfo>,
    ) -> ProviderResult<()> {
        if *self.should_fail.lock().unwrap() {
            return Err(katana_provider::ProviderError::Other(
                self.error_message.lock().unwrap().clone(),
            ));
        }

        self.blocks.lock().unwrap().push((block, states, receipts));
        Ok(())
    }
}

/// Helper function to create a minimal test block.
fn create_test_block(block_number: BlockNumber) -> StateUpdateWithBlock {
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

#[rstest]
#[case(100, 100, vec![100])]
#[case(100, 105, vec![100, 101, 102, 103, 104, 105])]
#[case(100, 110, vec![100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110])]
#[tokio::test]
async fn download_and_store_blocks(
    #[case] from_block: BlockNumber,
    #[case] to_block: BlockNumber,
    #[case] expected_blocks: Vec<BlockNumber>,
) {
    let provider = MockBlockWriter::new();
    let mut downloader = MockBlockDownloader::new();

    for block_num in from_block..=to_block {
        downloader = downloader.with_block(block_num, create_test_block(block_num));
    }

    let mut stage = Blocks::new(provider.clone(), downloader.clone());
    let input = StageExecutionInput { from: from_block, to: to_block };

    let result = stage.execute(&input).await;
    assert!(result.is_ok());

    // Verify download_blocks was called with the correct block numbers in the correct sequence
    assert_eq!(downloader.requested_blocks(), expected_blocks);
    // Verify insert_block_with_states_and_receipts was called with the correct block numbers in the
    // correct sequence
    assert_eq!(provider.stored_block_numbers(), expected_blocks);
}

#[tokio::test]
async fn download_failure_returns_error() {
    let block_number = 100;
    let error_msg = "Network error".to_string();

    let downloader = MockBlockDownloader::new().with_error(block_number, error_msg.clone());
    let provider = MockBlockWriter::new();

    let mut stage = Blocks::new(provider.clone(), downloader.clone());
    let input = StageExecutionInput { from: block_number, to: block_number };

    let result = stage.execute(&input).await;

    // Verify it's a Blocks error
    if let Err(err) = result {
        match err {
            katana_stage::Error::Blocks(e) => {
                assert!(e.to_string().contains(&error_msg))
            }
            _ => panic!("Expected Error::Blocks variant, got: {err:#?}"),
        }
    }

    // Verify download was attempted
    assert_eq!(downloader.requested_blocks(), vec![100]);
    // Verify no blocks were stored
    assert_eq!(provider.stored_block_count(), 0);
}

#[tokio::test]
async fn storage_failure_returns_error() {
    let block_number = 100;
    let test_block = create_test_block(block_number);
    let error_msg = "Storage full".to_string();

    let downloader = MockBlockDownloader::new().with_block(block_number, test_block);
    let provider = MockBlockWriter::new().with_insert_error(error_msg.clone());

    let mut stage = Blocks::new(provider.clone(), downloader.clone());
    let input = StageExecutionInput { from: block_number, to: block_number };

    let result = stage.execute(&input).await;

    // Verify it's a Blocks error
    if let Err(err) = result {
        match err {
            katana_stage::Error::Provider(e) => {
                assert!(e.to_string().contains(&error_msg))
            }
            _ => panic!("Expected Error::Provider variant, got: {err:#?}"),
        }
    }

    // Verify download was attempted
    assert_eq!(downloader.requested_blocks(), vec![100]);
    // Verify no blocks were stored
    assert_eq!(provider.stored_block_count(), 0);
}

// This test is only testing the debug sanity check in Blocks::execute(). Becase the
// `BlockDownloader` implementation could theoretically return whatever based on the block input
// because the input of `BlockDownloader::download_blocks` doesn't prohibit invalid block range.
// Maybe that's a good reason to change its method signature to `fn download_blocks(&self, range:
// Range<BlockNumber>)` ??
#[tokio::test]
#[ignore = "Stage input validation should be done on the `Pipeline` level"]
async fn empty_range_downloads_nothing() {
    // When from > to, the range is empty
    let downloader = MockBlockDownloader::new();
    let provider = MockBlockWriter::new();

    let mut stage = Blocks::new(provider.clone(), downloader.clone());
    let input = StageExecutionInput { from: 100, to: 99 };

    let result = stage.execute(&input).await;
    assert!(result.is_ok());

    // No downloads should occur for empty range
    assert_eq!(downloader.requested_blocks().len(), 0);
    assert_eq!(provider.stored_block_count(), 0);
}

#[tokio::test]
async fn partial_download_failure_stops_execution() {
    let from_block = 100;
    let to_block = 105;

    // Configure first 3 blocks to succeed, 4th to fail
    let mut downloader = MockBlockDownloader::new();
    for block_num in from_block..=102 {
        downloader = downloader.with_block(block_num, create_test_block(block_num));
    }
    downloader = downloader.with_error(103, "Block not found".to_string());

    let provider = MockBlockWriter::new();
    let mut stage = Blocks::new(provider.clone(), downloader.clone());

    let input = StageExecutionInput { from: from_block, to: to_block };
    let result = stage.execute(&input).await;

    // Should fail on block 103
    assert!(result.is_err());

    // Download was attempted
    assert_eq!(downloader.download_call_count(), 1);
}

// Integration test with real gateway (requires network)
#[tokio::test]
#[ignore = "require external network"]
async fn fetch_blocks_from_gateway() {
    let from_block = 308919;
    let to_block = from_block + 2;

    let provider = test_provider();
    let feeder_gateway = SequencerGateway::sepolia();
    let downloader = BatchBlockDownloader::new_gateway(feeder_gateway, 10);

    let mut stage = Blocks::new(&provider, downloader);

    let input = StageExecutionInput { from: from_block, to: to_block };
    stage.execute(&input).await.expect("failed to execute stage");

    // check provider storage
    let block_number = provider.latest_number().expect("failed to get latest block number");
    assert_eq!(block_number, to_block);
}
