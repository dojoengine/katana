use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use katana_primitives::block::{BlockHashOrNumber, BlockNumber, Header};
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::state::StateUpdates;
use katana_primitives::Felt;
use katana_provider::api::block::HeaderProvider;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::trie::TrieWriter;
use katana_provider::ProviderResult;
use katana_stage::trie::StateTrie;
use katana_stage::{Stage, StageExecutionInput};
use rstest::rstest;
use starknet::macros::short_string;
use starknet_types_core::hash::{Poseidon, StarkHash};

/// Mock provider implementation for testing StateTrie stage.
///
/// Provides configurable responses for headers, state updates, and trie operations.
#[derive(Clone)]
struct MockProvider {
    /// Map of block number to header.
    headers: Arc<Mutex<HashMap<BlockNumber, Header>>>,
    /// Map of block number to state update.
    state_updates: Arc<Mutex<HashMap<BlockNumber, StateUpdates>>>,
    /// Track trie insert calls for verification.
    trie_insert_calls: Arc<Mutex<Vec<BlockNumber>>>,
    /// Whether to return an error on trie operations.
    should_fail: Arc<Mutex<bool>>,
}

impl MockProvider {
    fn new() -> Self {
        Self {
            headers: Arc::new(Mutex::new(HashMap::new())),
            state_updates: Arc::new(Mutex::new(HashMap::new())),
            trie_insert_calls: Arc::new(Mutex::new(Vec::new())),
            should_fail: Arc::new(Mutex::new(false)),
        }
    }

    /// Configure a header for a specific block.
    fn with_header(self, block_number: BlockNumber, header: Header) -> Self {
        self.headers.lock().unwrap().insert(block_number, header);
        self
    }

    /// Configure a state update for a specific block.
    fn with_state_update(self, block_number: BlockNumber, state_update: StateUpdates) -> Self {
        self.state_updates.lock().unwrap().insert(block_number, state_update);
        self
    }

    /// Get all block numbers that had trie inserts called.
    fn trie_insert_blocks(&self) -> Vec<BlockNumber> {
        self.trie_insert_calls.lock().unwrap().clone()
    }
}

impl HeaderProvider for MockProvider {
    fn header(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Header>> {
        let block_number = match id {
            BlockHashOrNumber::Num(num) => num,
            BlockHashOrNumber::Hash(_) => {
                return Err(katana_provider::ProviderError::Other(
                    "Hash lookup not supported in mock".to_string(),
                ))
            }
        };

        Ok(self.headers.lock().unwrap().get(&block_number).cloned())
    }
}

impl StateUpdateProvider for MockProvider {
    fn state_update(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<StateUpdates>> {
        let block_number = match block_id {
            BlockHashOrNumber::Num(num) => num,
            BlockHashOrNumber::Hash(_) => {
                return Err(katana_provider::ProviderError::Other(
                    "Hash lookup not supported in mock".to_string(),
                ))
            }
        };

        Ok(self.state_updates.lock().unwrap().get(&block_number).cloned())
    }

    fn declared_classes(
        &self,
        _block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ClassHash, CompiledClassHash>>> {
        Ok(None)
    }

    fn deployed_contracts(
        &self,
        _block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<katana_primitives::ContractAddress, ClassHash>>> {
        Ok(None)
    }
}

impl TrieWriter for MockProvider {
    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        _updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt> {
        if *self.should_fail.lock().unwrap() {
            return Err(katana_provider::ProviderError::Other("Trie insert failed".to_string()));
        }

        self.trie_insert_calls.lock().unwrap().push(block_number);
        // Return a mock class trie root
        Ok(Felt::from(0x1234u64))
    }

    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        _state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        if *self.should_fail.lock().unwrap() {
            return Err(katana_provider::ProviderError::Other("Trie insert failed".to_string()));
        }

        self.trie_insert_calls.lock().unwrap().push(block_number);
        // Return a mock contract trie root
        Ok(Felt::from(0x5678u64))
    }
}

/// Helper function to compute the expected state root from mock trie roots.
fn compute_mock_state_root() -> Felt {
    let class_trie_root = Felt::from(0x1234u64);
    let contract_trie_root = Felt::from(0x5678u64);
    Poseidon::hash_array(&[short_string!("STARKNET_STATE_V0"), contract_trie_root, class_trie_root])
}

/// Helper function to create a test header with a given state root.
fn create_test_header(block_number: BlockNumber, state_root: Felt) -> Header {
    Header {
        number: block_number,
        state_root,
        parent_hash: Felt::ZERO,
        timestamp: block_number as u64,
        sequencer_address: Default::default(),
        l1_gas_prices: Default::default(),
        l2_gas_prices: Default::default(),
        l1_data_gas_prices: Default::default(),
        l1_da_mode: L1DataAvailabilityMode::Calldata,
        starknet_version: Default::default(),
        transaction_count: 0,
        events_count: 0,
        state_diff_length: 0,
        transactions_commitment: Felt::ZERO,
        events_commitment: Felt::ZERO,
        receipts_commitment: Felt::ZERO,
        state_diff_commitment: Felt::ZERO,
    }
}

/// Helper function to create a minimal test state update.
fn create_test_state_update() -> StateUpdates {
    StateUpdates {
        nonce_updates: Default::default(),
        storage_updates: Default::default(),
        deployed_contracts: Default::default(),
        replaced_classes: Default::default(),
        declared_classes: Default::default(),
        deprecated_declared_classes: Default::default(),
    }
}

#[rstest]
#[case(100, 100, vec![100])]
#[case(100, 102, vec![100, 101, 102])]
#[case(100, 105, vec![100, 101, 102, 103, 104, 105])]
#[tokio::test]
async fn verify_state_roots_success(
    #[case] from_block: BlockNumber,
    #[case] to_block: BlockNumber,
    #[case] expected_blocks: Vec<BlockNumber>,
) {
    let mut provider = MockProvider::new();

    // Configure blocks with correct state roots
    let correct_state_root = compute_mock_state_root();
    for num in from_block..=to_block {
        let header = create_test_header(num, correct_state_root);
        let state_update = create_test_state_update();
        provider = provider.with_header(num, header).with_state_update(num, state_update);
    }

    let input = StageExecutionInput::new(from_block, to_block);
    let result = StateTrie::new(provider.clone()).execute(&input).await;
    assert!(result.is_ok(), "Stage execution should succeed");

    // Verify that trie inserts were called for each block (twice per block: classes + contracts)
    let trie_calls = provider.trie_insert_blocks();
    assert_eq!(trie_calls.len(), expected_blocks.len() * 2);
}

#[tokio::test]
async fn state_root_mismatch_returns_error() {
    let block_number = 100;
    let correct_state_root = compute_mock_state_root();
    let wrong_state_root = Felt::from(0x9999u64);

    let header = create_test_header(block_number, wrong_state_root);
    let state_update = create_test_state_update();
    let provider = MockProvider::new()
        .with_header(block_number, header)
        .with_state_update(block_number, state_update);

    let mut stage = StateTrie::new(provider);
    let input = StageExecutionInput::new(block_number, block_number);

    let result = stage.execute(&input).await;

    // Verify it's a StateTrie error with state root mismatch
    assert!(result.is_err());
    if let Err(err) = result {
        match err {
            katana_stage::Error::StateTrie(e) => {
                let error_msg = e.to_string();
                assert!(
                    error_msg.contains("State root mismatch"),
                    "Expected state root mismatch error, got: {}",
                    error_msg
                );
                assert!(
                    error_msg.contains(&format!("{:#x}", wrong_state_root)),
                    "Error should contain expected state root"
                );
                assert!(
                    error_msg.contains(&format!("{:#x}", correct_state_root)),
                    "Error should contain computed state root"
                );
            }
            _ => panic!("Expected Error::StateTrie variant, got: {err:#?}"),
        }
    }
}
