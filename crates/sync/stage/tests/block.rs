use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};

use katana_gateway_client::Client as SequencerGateway;
use katana_gateway_types::{
    Block, BlockStatus, ConfirmedStateUpdate, StateDiff, StateUpdate, StateUpdateWithBlock,
};
use katana_primitives::block::{
    BlockHash, BlockNumber, FinalityStatus, Header, SealedBlock, SealedBlockWithStatus,
};
use katana_primitives::chain::ChainId;
use katana_primitives::class::ClassHash;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::version::StarknetVersion;
use katana_primitives::{felt, ContractAddress, Felt};
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, BlockWriter};
use katana_provider::api::state::{
    HistoricalStateRetentionProvider, StateFactoryProvider, StateProvider,
};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderError, ProviderFactory};
use katana_stage::blocks::hash::compute_hash;
use katana_stage::blocks::{BatchBlockDownloader, BlockData, BlockDownloader, Blocks};
use katana_stage::{PruneInput, Stage, StageExecutionInput};
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

fn create_provider_with_blocks(blocks: Vec<SealedBlockWithStatus>) -> DbProviderFactory {
    let provider_factory = DbProviderFactory::new_in_memory();
    let provider_mut = provider_factory.provider_mut();

    for block in blocks {
        provider_mut
            .insert_block_with_states_and_receipts(
                block,
                Default::default(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
    }

    provider_mut.commit().expect("failed to commit");
    provider_factory
}

fn create_provider_with_block_range(
    block_range: RangeInclusive<BlockNumber>,
    state_updates: BTreeMap<BlockNumber, StateUpdatesWithClasses>,
) -> DbProviderFactory {
    let provider_factory = DbProviderFactory::new_in_memory();
    let provider_mut = provider_factory.provider_mut();
    let blocks = create_stored_blocks(block_range);

    for block in blocks {
        let block_num = block.block.header.number;
        let state_updates = state_updates.get(&block_num).cloned().unwrap_or_default();

        provider_mut
            .insert_block_with_states_and_receipts(block, state_updates, Vec::new(), Vec::new())
            .unwrap();
    }

    provider_mut.commit().expect("failed to commit");
    provider_factory
}

fn state_updates_with_contract_changes(
    contract_address: ContractAddress,
    class_hash: ClassHash,
    nonce: Felt,
    storage_key: Felt,
    storage_value: Felt,
) -> StateUpdatesWithClasses {
    let mut state_updates = StateUpdates::default();
    state_updates.deployed_contracts.insert(contract_address, class_hash);
    state_updates.nonce_updates.insert(contract_address, nonce);
    state_updates
        .storage_updates
        .insert(contract_address, BTreeMap::from([(storage_key, storage_value)]));

    StateUpdatesWithClasses { state_updates, ..Default::default() }
}

/// Gets all stored block numbers from the provider by checking which blocks actually exist.
fn get_stored_block_numbers(
    provider: &DbProviderFactory,
    expected_range: std::ops::RangeInclusive<BlockNumber>,
) -> Vec<BlockNumber> {
    let p = provider.provider();
    expected_range.filter(|&num| p.block_hash_by_num(num).ok().flatten().is_some()).collect()
}

fn create_stored_blocks(block_range: RangeInclusive<BlockNumber>) -> Vec<SealedBlockWithStatus> {
    let mut blocks: Vec<SealedBlockWithStatus> = Vec::new();

    let offset = *block_range.start();
    for i in block_range {
        let idx = i.abs_diff(offset) as usize;
        let parent_hash = if idx == 0 { Felt::ZERO } else { blocks[idx - 1].block.hash };

        blocks.push(create_stored_block(i, parent_hash));
    }

    blocks
}

fn create_stored_block(block_number: BlockNumber, parent_hash: BlockHash) -> SealedBlockWithStatus {
    let header = Header {
        number: block_number,
        parent_hash,
        timestamp: 0,
        sequencer_address: ContractAddress::default(),
        l1_gas_prices: Default::default(),
        l1_data_gas_prices: Default::default(),
        l2_gas_prices: Default::default(),
        l1_da_mode: L1DataAvailabilityMode::Calldata,
        starknet_version: StarknetVersion::V0_13_4,
        state_root: Felt::ZERO,
        state_diff_commitment: Felt::ZERO,
        transactions_commitment: Felt::ZERO,
        receipts_commitment: Felt::ZERO,
        events_commitment: Felt::ZERO,
        transaction_count: 0,
        events_count: 0,
        state_diff_length: 0,
    };

    // the chain id is irrelevant here because the block is using starknet version 0.13.4
    // only block pre 0.7.0 uses chain id in the hash computation
    let hash = compute_hash(&header, &ChainId::SEPOLIA);

    SealedBlockWithStatus {
        block: SealedBlock { hash, header, body: Vec::new() },
        status: FinalityStatus::AcceptedOnL2,
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

#[tokio::test]
async fn prune_compacts_state_history_at_boundary() {
    let contract_address = ContractAddress::from(felt!("0x123"));
    let class_hash: ClassHash = felt!("0xCAFE");
    let nonce = felt!("0x7");
    let storage_key = felt!("0x99");
    let storage_value = felt!("0xBEEF");

    // Only block 1 has state updates; later blocks are unchanged.
    let mut updates_by_block = BTreeMap::new();
    updates_by_block.insert(
        1,
        state_updates_with_contract_changes(
            contract_address,
            class_hash,
            nonce,
            storage_key,
            storage_value,
        ),
    );

    let provider = create_provider_with_block_range(0..=8, updates_by_block);
    let mut stage = Blocks::new(
        provider.clone(),
        MockBlockDownloader::new(),
        ChainId::SEPOLIA,
        TaskManager::current().task_spawner(),
    );

    // keep_from = 5, prune range = [0, 5)
    let output = stage.prune(&PruneInput::new(8, Some(3), None)).await.expect("prune must succeed");
    assert!(output.pruned_count > 0, "expected prune to delete history entries");
    assert_eq!(provider.provider_mut().earliest_available_state_block().unwrap(), Some(5));

    // Historical reads in the retained window must still work due to anchor compaction.
    let retained_state = provider.provider().historical(7.into()).unwrap().unwrap();
    assert_eq!(retained_state.class_hash_of_contract(contract_address).unwrap(), Some(class_hash));
    assert_eq!(retained_state.nonce(contract_address).unwrap(), Some(nonce));
    assert_eq!(retained_state.storage(contract_address, storage_key).unwrap(), Some(storage_value));

    // Pruned historical state provider range should be unavailable.
    assert!(matches!(
        provider.provider().historical(2.into()),
        Err(ProviderError::HistoricalStatePruned { requested: 2, earliest_available: 5 })
    ));

    // Canonical per-block diffs remain exact even after history compaction.
    assert_eq!(
        provider.provider().state_update(1.into()).unwrap(),
        Some(
            state_updates_with_contract_changes(
                contract_address,
                class_hash,
                nonce,
                storage_key,
                storage_value,
            )
            .state_updates
        )
    );
    assert_eq!(provider.provider().state_update(5.into()).unwrap(), Some(StateUpdates::default()));
}

#[tokio::test]
async fn historical_returns_pruned_error_below_retention_boundary() {
    let provider = create_provider_with_block_range(0..=8, BTreeMap::new());

    let provider_mut = provider.provider_mut();
    provider_mut.set_earliest_available_state_block(5).unwrap();
    provider_mut.commit().unwrap();

    assert!(matches!(
        provider.provider().historical(4.into()),
        Err(ProviderError::HistoricalStatePruned { requested: 4, earliest_available: 5 })
    ));
    assert!(matches!(
        provider.provider().historical(3.into()),
        Err(ProviderError::HistoricalStatePruned { requested: 3, earliest_available: 5 })
    ));
    assert!(provider.provider().historical(5.into()).unwrap().is_some());
    assert!(provider.provider().historical(8.into()).unwrap().is_some());
}

#[tokio::test]
async fn blocks_prune_does_not_decrease_existing_retention_boundary() {
    let provider = create_provider_with_block_range(0..=8, BTreeMap::new());
    let mut stage = Blocks::new(
        provider.clone(),
        MockBlockDownloader::new(),
        ChainId::SEPOLIA,
        TaskManager::current().task_spawner(),
    );

    let provider_mut = provider.provider_mut();
    provider_mut.set_earliest_available_state_block(10).unwrap();
    provider_mut.commit().unwrap();

    // keep_from=5

    stage.prune(&PruneInput::new(8, Some(3), None)).await.expect("prune must succeed");
    assert_eq!(provider.provider_mut().earliest_available_state_block().unwrap(), Some(10));
}
