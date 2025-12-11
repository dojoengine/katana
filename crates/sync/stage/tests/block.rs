use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use katana_gateway_client::Client as SequencerGateway;
use katana_gateway_types::{
    Block, BlockStatus, ConfirmedStateUpdate, ErrorCode, GatewayError, StateDiff, StateUpdate,
    StateUpdateWithBlock,
};
use katana_primitives::block::{
    BlockHash, BlockNumber, FinalityStatus, Header, SealedBlock, SealedBlockWithStatus,
};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::Receipt;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::{felt, ContractAddress, Felt};
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, BlockWriter};
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
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
    ) -> impl Future<Output = Result<Vec<StateUpdateWithBlock>, katana_gateway_client::Error>> + Send
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
                        return Err(katana_gateway_client::Error::Sequencer(GatewayError {
                            code: ErrorCode::BlockNotFound,
                            message: error.clone(),
                            problems: None,
                        }))
                    }
                    None => {
                        return Err(katana_gateway_client::Error::Sequencer(GatewayError {
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

/// Creates a new in-memory provider with an initial block stored.
fn create_provider_with_block(block: SealedBlockWithStatus) -> DbProviderFactory {
    let provider_factory = DbProviderFactory::new_in_memory();
    let provider_mut = provider_factory.provider_mut();
    provider_mut
        .insert_block_with_states_and_receipts(block, Default::default(), Vec::new(), Vec::new())
        .expect("failed to insert initial block");
    provider_mut.commit().expect("failed to commit");
    provider_factory
}

/// Gets all stored block numbers from the provider by checking which blocks actually exist.
fn get_stored_block_numbers(
    provider: &DbProviderFactory,
    expected_range: std::ops::RangeInclusive<BlockNumber>,
) -> Vec<BlockNumber> {
    let p = provider.provider();
    expected_range.filter(|&num| p.block_hash_by_num(num).ok().flatten().is_some()).collect()
}

/// Helper function to create a minimal test `SealedBlockWithStatus`.
///
/// Creates a block with the given number and automatically sets the parent hash
/// to the hash of block N-1 (using `Felt::from(block_number - 1)`).
fn create_stored_block(block_number: BlockNumber) -> SealedBlockWithStatus {
    let header = Header {
        number: block_number,
        parent_hash: Felt::from(block_number.saturating_sub(1)),
        timestamp: block_number,
        sequencer_address: ContractAddress::default(),
        l1_gas_prices: Default::default(),
        l1_data_gas_prices: Default::default(),
        l2_gas_prices: Default::default(),
        l1_da_mode: L1DataAvailabilityMode::Calldata,
        starknet_version: Default::default(),
        state_root: Felt::ZERO,
        state_diff_commitment: Felt::ZERO,
        transactions_commitment: Felt::ZERO,
        receipts_commitment: Felt::ZERO,
        events_commitment: Felt::ZERO,
        transaction_count: 0,
        events_count: 0,
        state_diff_length: 0,
    };

    SealedBlockWithStatus {
        block: SealedBlock { hash: Felt::from(block_number), header, body: Vec::new() },
        status: FinalityStatus::AcceptedOnL2,
    }
}

/// Helper function to create a minimal test block. The created block has a parent hash == block
/// number - 1 for simplicity sake
fn create_downloaded_block(block_number: BlockNumber) -> StateUpdateWithBlock {
    create_downloaded_block_with_parent(block_number, Felt::from(block_number.saturating_sub(1)))
}

/// Helper function to create a test block with a specific parent hash.
fn create_downloaded_block_with_parent(
    block_number: BlockNumber,
    parent_hash: Felt,
) -> StateUpdateWithBlock {
    StateUpdateWithBlock {
        block: Block {
            status: BlockStatus::AcceptedOnL2,
            block_hash: Some(Felt::from(block_number)),
            parent_block_hash: parent_hash,
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
    let provider = create_provider_with_block(create_stored_block(from_block - 1));
    let mut downloader = MockBlockDownloader::new();

    for block_num in from_block..=to_block {
        downloader = downloader.with_block(block_num, create_downloaded_block(block_num));
    }

    let mut stage = Blocks::new(provider.clone(), downloader.clone());
    let input = StageExecutionInput::new(from_block, to_block);

    let result = stage.execute(&input).await;
    assert!(result.is_ok());

    // Verify download_blocks was called with the correct block numbers in the correct sequence
    assert_eq!(downloader.requested_blocks(), expected_blocks);
    // Verify blocks were stored correctly - should have initial block + downloaded blocks
    let stored = get_stored_block_numbers(&provider, (from_block - 1)..=to_block);
    assert_eq!(stored.len(), expected_blocks.len() + 1); // +1 for initial block
    assert_eq!(&stored[1..], expected_blocks.as_slice());
}

#[tokio::test]
async fn download_failure_returns_error() {
    let block_number = 100;
    let error_msg = "Network error".to_string();

    let downloader = MockBlockDownloader::new().with_error(block_number, error_msg.clone());
    // Create provider with initial block at block_number - 1
    let provider = create_provider_with_block(create_stored_block(block_number - 1));

    let mut stage = Blocks::new(provider.clone(), downloader.clone());
    let input = StageExecutionInput::new(block_number, block_number);

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
    // Verify only initial block was stored (no new blocks)
    let stored = get_stored_block_numbers(&provider, (block_number - 1)..=block_number);
    assert_eq!(stored.len(), 1); // Only the initial block
}

#[tokio::test]
async fn partial_download_failure_stops_execution() {
    let from_block = 100;
    let to_block = 105;

    // Configure first 3 blocks to succeed, 4th to fail
    let mut downloader = MockBlockDownloader::new();
    for block_num in from_block..=102 {
        downloader = downloader.with_block(block_num, create_downloaded_block(block_num));
    }
    downloader = downloader.with_error(103, "Block not found".to_string());

    let provider = create_provider_with_block(create_stored_block(from_block - 1));
    let mut stage = Blocks::new(provider.clone(), downloader.clone());

    let input = StageExecutionInput::new(from_block, to_block);
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

    // Create provider with initial block before the test range
    // The parent hash must match what the network returns for block from_block
    let provider = create_provider_with_block(create_stored_block(from_block - 1));
    let feeder_gateway = SequencerGateway::sepolia();
    let downloader = BatchBlockDownloader::new_gateway(feeder_gateway, 10);

    let mut stage = Blocks::new(provider.clone(), downloader);

    let input = StageExecutionInput::new(from_block, to_block);
    stage.execute(&input).await.expect("failed to execute stage");

    // check provider storage
    let block_number =
        provider.provider().latest_number().expect("failed to get latest block number");
    assert_eq!(block_number, to_block);
}

#[tokio::test]
async fn downloaded_blocks_do_not_form_valid_chain_with_stored_blocks() {
    use katana_stage::blocks;

    let provider = create_provider_with_block(create_stored_block(99));
    let downloader = MockBlockDownloader::new()
        .with_block(100, create_downloaded_block_with_parent(100, felt!("0x1337")));

    let mut stage = Blocks::new(provider.clone(), downloader.clone());
    let input = StageExecutionInput::new(100, 100);

    let result = stage.execute(&input).await;

    let expected_error = blocks::Error::ChainInvariantViolation {
        block_num: 100,
        parent_hash: felt!("0x1337"),
        expected_hash: felt!("99"),
    };

    // Should fail with chain invariant violation
    assert!(result.is_err());
    if let Err(err) = result {
        match err {
            katana_stage::Error::Blocks(e) => {
                assert_eq!(e.to_string(), expected_error.to_string());
            }
            _ => panic!("Expected Error::Blocks variant, got: {err:#?}"),
        }
    }

    // Verify no blocks were stored due to validation failure (except for block 99)
    let stored = get_stored_block_numbers(&provider, 99..=100);
    assert_eq!(stored.len(), 1);
}

#[tokio::test]
async fn downloaded_blocks_do_not_form_valid_chain() {
    use katana_stage::blocks;

    let provider = create_provider_with_block(create_stored_block(99));
    let downloader = MockBlockDownloader::new()
        .with_block(100, create_downloaded_block(100))
        .with_block(101, create_downloaded_block(101))
        // Create blocks where block 102 has an invalid parent hash
        // Block 102 with incorrect parent hash (should be 101)
        .with_block(102, create_downloaded_block_with_parent(102, Felt::from(999)));

    let mut stage = Blocks::new(provider.clone(), downloader.clone());
    let input = StageExecutionInput::new(100, 102);

    let result = stage.execute(&input).await;

    let expected_error = blocks::Error::ChainInvariantViolation {
        block_num: 102,
        parent_hash: felt!("999"),
        expected_hash: felt!("101"),
    };

    // Should fail with chain invariant violation
    assert!(result.is_err());
    if let Err(err) = result {
        match err {
            katana_stage::Error::Blocks(e) => {
                assert_eq!(e.to_string(), expected_error.to_string());
            }
            _ => panic!("Expected Error::Blocks variant, got: {err:#?}"),
        }
    }

    // Verify no blocks were stored due to validation failure (except for block 99)
    let stored = get_stored_block_numbers(&provider, 99..=102);
    assert_eq!(stored.len(), 1);
}
