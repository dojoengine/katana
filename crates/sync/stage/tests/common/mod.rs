use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use katana_primitives::block::{
    BlockHash, BlockNumber, FinalityStatus, Header, SealedBlock, SealedBlockWithStatus,
};
use katana_primitives::chain::ChainId;
use katana_primitives::class::ClassHash;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::version::StarknetVersion;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::block::{BlockHashProvider, BlockWriter};
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use katana_stage::blocks::hash::compute_hash;

pub fn create_provider_with_blocks(blocks: Vec<SealedBlockWithStatus>) -> DbProviderFactory {
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

pub fn create_provider_with_block_range(
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

pub fn state_updates_with_contract_changes(
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

/// Creates a provider with blocks stored via `insert_block_data` only (no state history).
///
/// This simulates the state after the `Blocks` stage runs but before `IndexHistory`,
/// so that `IndexHistory::execute` has block data and `BlockStateUpdates` to read from
/// but the history index tables are empty.
pub fn create_provider_with_block_data_only(
    block_range: RangeInclusive<BlockNumber>,
    state_updates: BTreeMap<BlockNumber, StateUpdatesWithClasses>,
) -> DbProviderFactory {
    let provider_factory = DbProviderFactory::new_in_memory();
    let provider_mut = provider_factory.provider_mut();
    let blocks = create_stored_blocks(block_range);

    for block in blocks {
        let block_num = block.block.header.number;
        let state_updates = state_updates.get(&block_num).cloned().unwrap_or_default();

        provider_mut.insert_block_data(block, state_updates, Vec::new(), Vec::new()).unwrap();
    }

    provider_mut.commit().expect("failed to commit");
    provider_factory
}

/// Gets all stored block numbers from the provider by checking which blocks actually exist.
pub fn get_stored_block_numbers(
    provider: &DbProviderFactory,
    expected_range: std::ops::RangeInclusive<BlockNumber>,
) -> Vec<BlockNumber> {
    let p = provider.provider();
    expected_range.filter(|&num| p.block_hash_by_num(num).ok().flatten().is_some()).collect()
}

pub fn create_stored_blocks(
    block_range: RangeInclusive<BlockNumber>,
) -> Vec<SealedBlockWithStatus> {
    let mut blocks: Vec<SealedBlockWithStatus> = Vec::new();

    let offset = *block_range.start();
    for i in block_range {
        let idx = i.abs_diff(offset) as usize;
        let parent_hash = if idx == 0 { Felt::ZERO } else { blocks[idx - 1].block.hash };

        blocks.push(create_stored_block(i, parent_hash));
    }

    blocks
}

pub fn create_stored_block(
    block_number: BlockNumber,
    parent_hash: BlockHash,
) -> SealedBlockWithStatus {
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
