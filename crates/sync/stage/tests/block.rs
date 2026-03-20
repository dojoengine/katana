#[allow(unused)]
mod common;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use common::{
    create_provider_with_block_range, create_provider_with_blocks, create_stored_block,
    get_stored_block_numbers,
};
use katana_gateway_client::Client as SequencerGateway;
use katana_gateway_types::{
    Block, BlockStatus, ConfirmedStateUpdate, StateDiff, StateUpdate, StateUpdateWithBlock,
};
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::chain::ChainId;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::{felt, ContractAddress, Felt};
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::ProviderFactory;
use katana_stage::blocks::hash::compute_hash;
use katana_stage::blocks::{BatchBlockDownloader, BlockData, BlockDownloader, Blocks};
use katana_stage::{Stage, StageExecutionInput};
use katana_tasks::TaskManager;
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

    fn with_blocks<I>(self, iter: I) -> Self
    where
        I: IntoIterator<Item = (BlockNumber, StateUpdateWithBlock)>,
    {
        self.responses.lock().unwrap().extend(iter.into_iter().map(|(k, v)| (k, Ok(v))));
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

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct MockError(String);

// We're only testing the stage business logic so we don't really care about using the
// BatchDownloader/Downloader combination.
impl BlockDownloader for MockBlockDownloader {
    type Error = MockError;

    fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> impl Future<Output = Result<Vec<BlockData>, Self::Error>> + Send {
        async move {
            let block_numbers: Vec<BlockNumber> = (from..=to).collect();

            // Track the call
            self.download_calls.lock().unwrap().push(block_numbers.clone());

            let mut results = Vec::new();
            let responses = self.responses.lock().unwrap();

            for block_num in block_numbers {
                match responses.get(&block_num) {
                    Some(Ok(block_data)) => results.push(BlockData::from(block_data.clone())),
                    Some(Err(error)) => {
                        return Err(MockError(error.clone()));
                    }
                    None => {
                        return Err(MockError(format!(
                            "No response configured for block {}",
                            block_num
                        )));
                    }
                }
            }

            Ok(results)
        }
    }
}

fn create_downloaded_block(block_number: BlockNumber, parent_hash: Felt) -> StateUpdateWithBlock {
    let mut downloaded_block = StateUpdateWithBlock {
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
            receipt_commitment: Some(Felt::ZERO),
            event_commitment: Some(Felt::ZERO),
            state_diff_commitment: Some(Felt::ZERO),
            state_diff_length: None,
            state_root: Some(Felt::ZERO),
        },
        state_update: StateUpdate::Confirmed(ConfirmedStateUpdate {
            block_hash: Felt::from(block_number),
            new_root: Felt::ZERO,
            old_root: Felt::ZERO,
            state_diff: StateDiff::default(),
        }),
    };

    let block: BlockData = downloaded_block.clone().into();
    let actual_block_hash = compute_hash(&block.block.block.header, &ChainId::SEPOLIA);

    downloaded_block.block.block_hash = Some(actual_block_hash);
    downloaded_block
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
    let genesis = create_stored_block(from_block - 1, BlockHash::ZERO);
    let mut downloaded_blocks: Vec<(BlockNumber, StateUpdateWithBlock)> = Vec::new();

    for i in from_block..=to_block {
        let idx = (i % from_block) as usize;

        let parent_hash = if idx == 0 {
            genesis.block.hash
        } else {
            downloaded_blocks[idx - 1].1.block.block_hash.unwrap()
        };

        downloaded_blocks.push((i, create_downloaded_block(i, parent_hash)));
    }

    let provider = create_provider_with_blocks(vec![genesis]);
    let downloader = MockBlockDownloader::new().with_blocks(downloaded_blocks);

    let mut stage = Blocks::new(
        provider.clone(),
        downloader.clone(),
        ChainId::SEPOLIA,
        TaskManager::current().task_spawner(),
    );
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

    // Create provider with initial block number 99
    let genesis = create_stored_block(99, BlockHash::ZERO);
    let provider = create_provider_with_blocks(vec![genesis]);

    let downloader = MockBlockDownloader::new().with_error(block_number, error_msg.clone());

    let mut stage = Blocks::new(
        provider.clone(),
        downloader.clone(),
        ChainId::SEPOLIA,
        TaskManager::current().task_spawner(),
    );
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
    let mut downloaded_blocks: Vec<(BlockNumber, StateUpdateWithBlock)> = Vec::new();

    for block_num in from_block..=102 {
        let idx = (block_num % from_block) as usize;

        let parent_hash = if idx == 0 {
            BlockHash::ZERO
        } else {
            downloaded_blocks[idx - 1].1.block.block_hash.unwrap()
        };

        let block = create_downloaded_block(block_num, parent_hash);
        downloaded_blocks.push((block_num, block));
    }

    let downloader = MockBlockDownloader::new()
        .with_blocks(downloaded_blocks)
        .with_error(103, "Block not found".to_string());

    let provider = create_provider_with_block_range(99..=99, Default::default());
    let mut stage = Blocks::new(
        provider.clone(),
        downloader.clone(),
        ChainId::SEPOLIA,
        TaskManager::current().task_spawner(),
    );

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

    let genesis = create_stored_block(from_block - 1, BlockHash::ZERO);
    let provider = create_provider_with_blocks(vec![genesis]);

    let feeder_gateway = SequencerGateway::sepolia();
    let downloader = BatchBlockDownloader::new_gateway(feeder_gateway, 10);

    let mut stage = Blocks::new(
        provider.clone(),
        downloader,
        ChainId::SEPOLIA,
        TaskManager::current().task_spawner(),
    );

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

    let genesis = create_stored_block(99, BlockHash::ZERO);
    let expected_parent_hash = genesis.block.hash;

    let provider = create_provider_with_blocks(vec![genesis]);

    // download a block with an invalid parent hash
    let block1 = create_downloaded_block(100, felt!("0x1337"));
    let downloader = MockBlockDownloader::new().with_block(100, block1);

    let mut stage = Blocks::new(
        provider.clone(),
        downloader.clone(),
        ChainId::SEPOLIA,
        TaskManager::current().task_spawner(),
    );
    let input = StageExecutionInput::new(100, 100);

    let result = stage.execute(&input).await;

    let expected_error = blocks::Error::ChainInvariantViolation {
        block_num: 100,
        parent_hash: felt!("0x1337"),
        expected_hash: expected_parent_hash,
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

    let genesis = create_stored_block(99, BlockHash::ZERO);
    let block1 = create_downloaded_block(100, genesis.block.hash);
    let block2 = create_downloaded_block(101, block1.block.block_hash.unwrap());

    let expected_prev_block_hash = block2.block.block_hash.clone().unwrap();

    let provider = create_provider_with_blocks(vec![genesis]);
    let downloader = MockBlockDownloader::new()
        .with_block(100, block1)
        .with_block(101, block2)
        // block 102 has an invalid parent hash
        .with_block(102, create_downloaded_block(102, Felt::from(999)));

    let mut stage = Blocks::new(
        provider.clone(),
        downloader.clone(),
        ChainId::SEPOLIA,
        TaskManager::current().task_spawner(),
    );
    let input = StageExecutionInput::new(100, 102);

    let result = stage.execute(&input).await;

    let expected_error = blocks::Error::ChainInvariantViolation {
        block_num: 102,
        parent_hash: felt!("999"),
        expected_hash: expected_prev_block_hash,
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
