use anyhow::Result;
use katana_primitives::block::{
    Block, BlockHashOrNumber, BlockNumber, BlockWithTxHashes, FinalityStatus,
};
use katana_primitives::env::BlockEnv;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::transaction::TxWithHash;
use katana_provider::api::block::{
    BlockHashProvider, BlockNumberProvider, BlockProvider, BlockStatusProvider, BlockWriter,
};
use katana_provider::api::env::BlockEnvProvider;
use katana_provider::api::state::{StateFactoryProvider, StateRootProvider};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider, TransactionTraceProvider,
};
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use rstest_reuse::{self, *};

mod fixtures;
mod utils;

use fixtures::{db_provider, mock_state_updates};
use katana_primitives::Felt;

use crate::fixtures::db_provider_with_states;

#[apply(insert_block_cases)]
fn insert_block_with_db_provider(
    #[from(db_provider)] provider_factory: DbProviderFactory,
    #[case] block_count: u64,
) -> Result<()> {
    insert_block_test_impl(provider_factory, 0, block_count)
}

#[apply(insert_block_cases)]
fn insert_block_empty_with_db_provider(
    #[from(db_provider)] provider_factory: DbProviderFactory,
    #[case] block_count: u64,
) -> Result<()> {
    insert_block_empty_test_impl(provider_factory, 0, block_count)
}

fn insert_block_test_impl<P>(
    provider_factory: P,
    start: BlockNumber,
    end: BlockNumber,
) -> Result<()>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: BlockProvider
        + ReceiptProvider
        + StateFactoryProvider
        + TransactionStatusProvider
        + TransactionTraceProvider
        + BlockEnvProvider,
    <P as ProviderFactory>::ProviderMut: BlockWriter,
{
    let blocks = utils::generate_dummy_blocks_and_receipts(start, end);

    for (block, receipts, executions) in &blocks {
        let povider_mut = provider_factory.provider_mut();
        povider_mut.insert_block_with_states_and_receipts(
            block.clone(),
            Default::default(),
            receipts.clone(),
            executions.clone(),
        )?;
        povider_mut.commit().unwrap();

        let provider = provider_factory.provider();
        assert_eq!(provider.latest_number().unwrap(), block.block.header.number);
        assert_eq!(provider.latest_hash().unwrap(), block.block.hash);
    }

    let provider = provider_factory.provider();

    let actual_blocks_in_range = provider.blocks_in_range(start..=end)?;

    assert_eq!(actual_blocks_in_range.len(), (end - start + 1) as usize); // bcs the start and end is inclusive
    assert_eq!(
        actual_blocks_in_range,
        blocks.clone().into_iter().map(|b| b.0.block.unseal()).collect::<Vec<Block>>()
    );

    for (block, receipts, executions) in blocks {
        let block_id = BlockHashOrNumber::Hash(block.block.hash);

        let expected_block_num = block.block.header.number;
        let expected_block_hash = block.block.hash;
        let expected_block = block.block.unseal();

        let expected_block_env = BlockEnv {
            number: expected_block_num,
            timestamp: expected_block.header.timestamp,
            starknet_version: expected_block.header.starknet_version,
            l2_gas_prices: expected_block.header.l2_gas_prices.clone(),
            l1_gas_prices: expected_block.header.l1_gas_prices.clone(),
            l1_data_gas_prices: expected_block.header.l1_data_gas_prices.clone(),
            sequencer_address: expected_block.header.sequencer_address,
        };

        let actual_block_hash = provider.block_hash_by_num(expected_block_num)?;

        let actual_block = provider.block(block_id)?;
        let actual_block_txs = provider.transactions_by_block(block_id)?;
        let actual_status = provider.block_status(block_id)?;
        let actual_state_root =
            provider.historical(block_id)?.map(|s| s.state_root()).transpose()?;

        let actual_block_tx_count = provider.transaction_count_by_block(block_id)?;
        let actual_receipts = provider.receipts_by_block(block_id)?;
        let actual_executions = provider.transaction_executions_by_block(block_id)?;

        let expected_block_with_tx_hashes = BlockWithTxHashes {
            header: expected_block.header.clone(),
            body: expected_block.body.clone().into_iter().map(|t| t.hash).collect(),
        };

        let actual_block_with_tx_hashes = provider.block_with_tx_hashes(block_id)?;
        let actual_block_env = provider.block_env_at(block_id)?;

        assert_eq!(actual_status, Some(FinalityStatus::AcceptedOnL2));
        assert_eq!(actual_block_with_tx_hashes, Some(expected_block_with_tx_hashes));

        for (idx, tx) in expected_block.body.iter().enumerate() {
            let actual_receipt = provider.receipt_by_hash(tx.hash)?;
            let actual_execution = provider.transaction_execution(tx.hash)?;
            let actual_tx = provider.transaction_by_hash(tx.hash)?;
            let actual_tx_status = provider.transaction_status(tx.hash)?;
            let actual_tx_block_num_hash = provider.transaction_block_num_and_hash(tx.hash)?;
            let actual_tx_by_block_idx =
                provider.transaction_by_block_and_idx(block_id, idx as u64)?;

            assert_eq!(actual_tx_block_num_hash, Some((expected_block_num, expected_block_hash)));
            assert_eq!(actual_tx_status, Some(FinalityStatus::AcceptedOnL2));
            assert_eq!(actual_receipt, Some(receipts[idx].clone()));
            assert_eq!(actual_execution, Some(executions[idx].clone()));
            assert_eq!(actual_tx_by_block_idx, Some(tx.clone()));
            assert_eq!(actual_tx, Some(tx.clone()));
        }

        assert_eq!(actual_block_env, Some(expected_block_env));

        assert_eq!(actual_receipts.as_ref().map(|r| r.len()), Some(expected_block.body.len()));
        assert_eq!(actual_receipts, Some(receipts));
        assert_eq!(actual_executions, Some(executions));

        assert_eq!(actual_block_tx_count, Some(expected_block.body.len() as u64));
        assert_eq!(actual_state_root, Some(expected_block.header.state_root));
        assert_eq!(actual_block_txs, Some(expected_block.body.clone()));
        assert_eq!(actual_block_hash, Some(expected_block_hash));
        assert_eq!(actual_block, Some(expected_block));
    }

    Ok(())
}

fn insert_block_empty_test_impl<P>(
    storage_provider: P,
    start: BlockNumber,
    end: BlockNumber,
) -> Result<()>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: BlockProvider
        + ReceiptProvider
        + StateFactoryProvider
        + TransactionStatusProvider
        + TransactionTraceProvider
        + BlockEnvProvider,
    <P as ProviderFactory>::ProviderMut: BlockWriter,
{
    let blocks = utils::generate_dummy_blocks_empty(start, end);
    let txs: Vec<TxWithHash> = blocks.iter().flat_map(|block| block.block.body.clone()).collect();

    let total_txs = txs.len() as u64;
    assert_eq!(total_txs, 0);

    for block in &blocks {
        let provider_mut = storage_provider.provider_mut();
        provider_mut.insert_block_with_states_and_receipts(
            block.clone(),
            Default::default(),
            vec![],
            vec![],
        )?;
        provider_mut.commit().unwrap();

        let provider = storage_provider.provider();
        assert_eq!(provider.latest_number().unwrap(), block.block.header.number);
        assert_eq!(provider.latest_hash().unwrap(), block.block.hash);
    }

    let provider = storage_provider.provider();
    let actual_blocks_in_range = provider.blocks_in_range(start..=end)?;

    assert_eq!(actual_blocks_in_range.len(), (end - start + 1) as usize); // because the start and end are inclusive
    assert_eq!(
        actual_blocks_in_range,
        blocks.clone().into_iter().map(|b| b.block.unseal()).collect::<Vec<Block>>()
    );

    for block in blocks {
        let block_id = BlockHashOrNumber::Hash(block.block.hash);

        let expected_block_num = block.block.header.number;
        let expected_block_hash = block.block.hash;
        let expected_block = block.block.unseal();

        let expected_block_env = BlockEnv {
            number: expected_block_num,
            timestamp: expected_block.header.timestamp,
            starknet_version: expected_block.header.starknet_version,
            l2_gas_prices: expected_block.header.l2_gas_prices.clone(),
            l1_gas_prices: expected_block.header.l1_gas_prices.clone(),
            l1_data_gas_prices: expected_block.header.l1_data_gas_prices.clone(),
            sequencer_address: expected_block.header.sequencer_address,
        };

        let actual_block_hash = provider.block_hash_by_num(expected_block_num)?;

        let actual_block = provider.block(block_id)?;
        let actual_block_txs = provider.transactions_by_block(block_id)?;
        let actual_status = provider.block_status(block_id)?;
        let actual_state_root =
            provider.historical(block_id)?.map(|s| s.state_root()).transpose()?;

        let actual_block_tx_count = provider.transaction_count_by_block(block_id)?;
        let actual_receipts = provider.receipts_by_block(block_id)?;
        let actual_executions = provider.transaction_executions_by_block(block_id)?;

        let expected_block_with_tx_hashes =
            BlockWithTxHashes { header: expected_block.header.clone(), body: vec![] };

        let actual_block_with_tx_hashes = provider.block_with_tx_hashes(block_id)?;
        let actual_block_env = provider.block_env_at(block_id)?;

        assert_eq!(actual_status, Some(FinalityStatus::AcceptedOnL2));
        assert_eq!(actual_block_with_tx_hashes, Some(expected_block_with_tx_hashes));

        let tx_hash = Felt::ZERO;

        let actual_receipt = provider.receipt_by_hash(tx_hash)?;
        let actual_execution = provider.transaction_execution(tx_hash)?;
        let actual_tx = provider.transaction_by_hash(tx_hash)?;
        let actual_tx_status = provider.transaction_status(tx_hash)?;
        let actual_tx_block_num_hash = provider.transaction_block_num_and_hash(tx_hash)?;
        let actual_tx_by_block_idx = provider.transaction_by_block_and_idx(block_id, 0)?;

        assert_eq!(actual_tx_block_num_hash, None);
        assert_eq!(actual_tx_status, None);
        assert_eq!(actual_receipt, None);
        assert_eq!(actual_execution, None);
        assert_eq!(actual_tx_by_block_idx, None);
        assert_eq!(actual_tx, None);

        assert_eq!(actual_block_env, Some(expected_block_env));

        assert_eq!(actual_receipts.as_ref().map(|r| r.len()), Some(expected_block.body.len()));
        assert_eq!(actual_receipts, Some(vec![]));
        assert_eq!(actual_executions, Some(vec![]));

        assert_eq!(actual_block_tx_count, Some(expected_block.body.len() as u64));
        assert_eq!(actual_state_root, Some(expected_block.header.state_root));
        assert_eq!(actual_block_txs, Some(expected_block.body.clone()));
        assert_eq!(actual_block_hash, Some(expected_block_hash));
        assert_eq!(actual_block, Some(expected_block));
    }

    Ok(())
}

#[apply(test_read_state_update)]
fn test_read_state_update_with_db_provider(
    #[from(db_provider_with_states)] provider_factory: DbProviderFactory,
    #[case] block_num: BlockNumber,
    #[case] expected_state_update: StateUpdatesWithClasses,
) -> Result<()> {
    let provider = provider_factory.provider();
    test_read_state_update_impl(provider, block_num, expected_state_update)
}

fn test_read_state_update_impl(
    provider: impl StateUpdateProvider,
    block_num: BlockNumber,
    expected_state_update: StateUpdatesWithClasses,
) -> Result<()> {
    let actual_state_update = provider.state_update(BlockHashOrNumber::from(block_num))?;
    assert_eq!(actual_state_update, Some(expected_state_update.state_updates));
    Ok(())
}

#[template]
#[rstest::rstest]
#[case::insert_1_block(1)]
#[case::insert_2_block(2)]
#[case::insert_5_block(5)]
#[case::insert_10_block(10)]
fn insert_block_cases(#[case] block_count: u64) {}

#[template]
#[rstest::rstest]
#[case::state_update_at_block_1(1, mock_state_updates()[0].clone())]
#[case::state_update_at_block_2(2, mock_state_updates()[1].clone())]
#[case::state_update_at_block_3(3, StateUpdatesWithClasses::default())]
#[case::state_update_at_block_5(5, mock_state_updates()[2].clone())]
fn test_read_state_update(
    #[case] block_num: BlockNumber,
    #[case] expected_state_update: StateUpdatesWithClasses,
) {
}

mod fork {
    use fixtures::fork::{fork_provider, fork_provider_with_spawned_fork_network_and_states};
    use katana_provider::ForkProviderFactory;

    use super::*;

    #[template]
    #[rstest::rstest]
    #[case::insert_1_block(1)]
    #[case::insert_2_block(2)]
    #[case::insert_5_block(5)]
    #[case::insert_10_block(10)]
    fn fork_insert_block_cases(#[case] block_count: u64) {}

    #[apply(fork_insert_block_cases)]
    fn insert_block_with_fork_provider(
        #[from(fork_provider)] provider_factory: ForkProviderFactory,
        #[case] block_count: u64,
    ) -> Result<()> {
        let forked_block = provider_factory.block();
        insert_block_test_impl(provider_factory, forked_block + 1, forked_block + block_count)
    }

    #[apply(fork_insert_block_cases)]
    fn insert_block_empty_with_fork_provider(
        #[from(fork_provider)] provider_factory: ForkProviderFactory,
        #[case] block_count: u64,
    ) -> Result<()> {
        let forked_block = provider_factory.block();
        insert_block_empty_test_impl(provider_factory, forked_block + 1, forked_block + block_count)
    }

    #[apply(test_read_state_update)]
    fn test_read_state_update_with_fork_provider(
        #[from(fork_provider_with_spawned_fork_network_and_states)]
        provider_factory: ForkProviderFactory,
        #[case] block_num: BlockNumber,
        #[case] expected_state_update: StateUpdatesWithClasses,
    ) -> Result<()> {
        let provider = provider_factory.provider();
        test_read_state_update_impl(provider, block_num, expected_state_update)
    }
}
