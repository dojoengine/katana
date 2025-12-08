use anyhow::Result;
use assert_matches::assert_matches;
use cainome::rs::abigen_legacy;
use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_node::config::fork::ForkingConfig;
use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber};
use katana_primitives::chain::NamedChainId;
use katana_primitives::event::MaybeForkedContinuationToken;
use katana_primitives::transaction::TxHash;
use katana_primitives::{felt, Felt};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_types::{
    BlockNumberResponse, EventFilter, GetBlockWithReceiptsResponse, GetBlockWithTxHashesResponse,
    MaybePreConfirmedBlock,
};
use katana_utils::node::ForkTestNode;
use katana_utils::TestNode;
use url::Url;

mod common;

const SEPOLIA_CHAIN_ID: Felt = NamedChainId::SN_SEPOLIA;
const SEPOLIA_URL: &str = "https://api.cartridge.gg/x/starknet/sepolia";
const FORK_BLOCK_NUMBER: BlockNumber = 268_471;
const FORK_BLOCK_HASH: BlockHash =
    felt!("0x208950cfcbba73ecbda1c14e4d58d66a8d60655ea1b9dcf07c16014ae8a93cd");

fn forking_cfg() -> ForkingConfig {
    ForkingConfig { url: Url::parse(SEPOLIA_URL).unwrap(), block: Some(FORK_BLOCK_NUMBER.into()) }
}

type LocalTestVector = Vec<((BlockNumber, BlockHash), TxHash)>;

/// A helper function for setting a test environment, forked from the SN_SEPOLIA chain.
/// This function will forked Sepolia at block [`FORK_BLOCK_NUMBER`] and create 10 blocks, each has
/// a single transaction.
///
/// The returned [`TestVector`] is a list of all the locally created blocks and transactions.
async fn setup_test_inner(no_mining: bool) -> (ForkTestNode, StarknetClient, LocalTestVector) {
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = no_mining;
    config.forking = Some(forking_cfg());

    let sequencer = TestNode::new_forked_with_config(config).await;
    let provider = sequencer.starknet_rpc_client();

    let mut txs_vector: LocalTestVector = Vec::new();

    // create some emtpy blocks and dummy transactions
    abigen_legacy!(Erc20Contract, "crates/contracts/build/legacy/erc20.json", derives(Clone));
    let contract = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), sequencer.account());

    if no_mining {
        // In no mining mode, bcs we're not producing any blocks, the transactions that we send
        // will all be included in the same block (pending).
        for _ in 1..=10 {
            let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };
            let res = contract.transfer(&Felt::ONE, &amount).send().await.unwrap();
            katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();

            // events in pending block doesn't have block hash and number, so we can safely put
            // dummy values here.
            txs_vector.push(((0, Felt::ZERO), res.transaction_hash));
        }
    } else {
        // We're in auto mining, each transaction will create a new block
        for i in 1..=10 {
            let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };
            let res = contract.transfer(&Felt::ONE, &amount).send().await.unwrap();
            let _ = katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();

            let block_num = (FORK_BLOCK_NUMBER + 1) + i; // plus 1 because fork genesis is FORK_BLOCK_NUMBER + 1

            let block_id = BlockIdOrTag::Number(block_num);
            let block = provider.get_block_with_tx_hashes(block_id).await.unwrap();
            let block_hash = match block {
                GetBlockWithTxHashesResponse::Block(b) => {
                    assert_eq!(b.transactions.len(), 1);
                    b.block_hash
                }

                _ => panic!("Expected a block"),
            };

            txs_vector.push((((FORK_BLOCK_NUMBER + 1) + i, block_hash), res.transaction_hash));
        }
    }

    (sequencer, provider, txs_vector)
}

async fn setup_test() -> (ForkTestNode, StarknetClient, LocalTestVector) {
    setup_test_inner(false).await
}

async fn setup_test_pending() -> (ForkTestNode, StarknetClient, LocalTestVector) {
    setup_test_inner(true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn can_fork() -> Result<()> {
    let (_sequencer, provider, _) = setup_test().await;

    let BlockNumberResponse { block_number } = provider.block_number().await?;
    let chain = provider.chain_id().await?;

    assert_eq!(chain, SEPOLIA_CHAIN_ID);
    assert_eq!(block_number, FORK_BLOCK_NUMBER + 11); // fork block + genesis + 10 blocks

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_blocks_from_num() -> Result<()> {
    let (_sequencer, provider, local_only_block) = setup_test().await;

    // -----------------------------------------------------------------------
    // Get the forked block
    // https://sepolia.voyager.online/block/0x208950cfcbba73ecbda1c14e4d58d66a8d60655ea1b9dcf07c16014ae8a93cd

    let num = FORK_BLOCK_NUMBER; // 268471
    let id = BlockIdOrTag::Number(num);

    let block = provider.get_block_with_txs(id).await?;
    assert_matches!(block, MaybePreConfirmedBlock::Confirmed(b) if b.block_number == num);

    let block = provider.get_block_with_receipts(id).await?;
    assert_matches!(block, GetBlockWithReceiptsResponse::Block(b) if b.block_number == num);

    let block = provider.get_block_with_tx_hashes(id).await?;
    assert_matches!(block, GetBlockWithTxHashesResponse::Block(b) if b.block_number == num);

    let result = provider.get_block_transaction_count(id).await;
    assert!(result.is_ok());

    // TODO: uncomment this once we include genesis forked state update
    // let state = provider.get_state_update(id).await?;
    // assert_matches!(state, starknet::core::types::MaybePendingStateUpdate::Update(_));

    // -----------------------------------------------------------------------
    // Get a block before the forked block

    // https://sepolia.voyager.online/block/0x42dc67af5003d212ac6cd784e72db945ea4d619898f30f422358ff215cbe1e4
    let num = FORK_BLOCK_NUMBER - 5; // 268466
    let id = BlockIdOrTag::Number(num);

    let block = provider.get_block_with_txs(id).await?;
    assert_matches!(block, MaybePreConfirmedBlock::Confirmed(b) if b.block_number == num);

    let block = provider.get_block_with_receipts(id).await?;
    assert_matches!(block, GetBlockWithReceiptsResponse::Block(b) if b.block_number == num);

    let block = provider.get_block_with_tx_hashes(id).await?;
    assert_matches!(block, GetBlockWithTxHashesResponse::Block(b) if b.block_number == num);

    let result = provider.get_block_transaction_count(id).await;
    assert!(result.is_ok());

    // TODO: uncomment this once we include genesis forked state update
    // let state = provider.get_state_update(id).await?;
    // assert_matches!(state, starknet::core::types::MaybePendingStateUpdate::Update(_));

    // -----------------------------------------------------------------------
    // Get a block that is locally generated

    for ((num, _), _) in local_only_block {
        let id = BlockIdOrTag::Number(num);

        let block = provider.get_block_with_txs(id).await?;
        assert_matches!(block, MaybePreConfirmedBlock::Confirmed(b) if b.block_number == num);

        let block = provider.get_block_with_receipts(id).await?;
        assert_matches!(block, GetBlockWithReceiptsResponse::Block(b) if b.block_number == num);

        let block = provider.get_block_with_tx_hashes(id).await?;
        assert_matches!(block, GetBlockWithTxHashesResponse::Block(b) if b.block_number == num);

        let count = provider.get_block_transaction_count(id).await?;
        assert_eq!(count, 1, "all the locally generated blocks should have 1 tx");

        // TODO: uncomment this once we include genesis forked state update
        // let state = provider.get_state_update(id).await?;
        // assert_matches!(state, starknet::core::types::MaybePendingStateUpdate::Update(_));
    }

    // -----------------------------------------------------------------------
    // Get a block that only exist in the forked chain

    // https://sepolia.voyager.online/block/0x347a9fa25700e7a2d8f26b39c0ecf765be9a78c559b9cae722a659f25182d10
    // We only created 10 local blocks so this is fine.
    let id = BlockIdOrTag::Number(270_328);
    let result = provider.get_block_with_txs(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_with_receipts(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_with_tx_hashes(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_transaction_count(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_state_update(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    // -----------------------------------------------------------------------
    // Get block that doesn't exist on the both the forked and local chain

    let id = BlockIdOrTag::Number(i64::MAX as u64);
    let result = provider.get_block_with_txs(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_with_receipts(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_with_tx_hashes(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_transaction_count(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_state_update(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_blocks_from_hash() {
    let (_sequencer, provider, local_only_block) = setup_test().await;

    // -----------------------------------------------------------------------
    // Get the forked block

    // https://sepolia.voyager.online/block/0x208950cfcbba73ecbda1c14e4d58d66a8d60655ea1b9dcf07c16014ae8a93cd
    let hash = felt!("0x208950cfcbba73ecbda1c14e4d58d66a8d60655ea1b9dcf07c16014ae8a93cd"); // 268471
    let id = BlockIdOrTag::Hash(hash);

    let block = provider.get_block_with_txs(id).await.unwrap();
    assert_matches!(block, MaybePreConfirmedBlock::Confirmed(b) if b.block_hash == hash);

    let block = provider.get_block_with_receipts(id).await.unwrap();
    assert_matches!(block, GetBlockWithReceiptsResponse::Block(b) if b.block_hash == hash);

    let block = provider.get_block_with_tx_hashes(id).await.unwrap();
    assert_matches!(block, GetBlockWithTxHashesResponse::Block(b) if b.block_hash == hash);

    let result = provider.get_block_transaction_count(id).await;
    assert!(result.is_ok());

    // TODO: uncomment this once we include genesis forked state update
    // let state = provider.get_state_update(id).await.unwrap();
    // assert_matches!(state, starknet::core::types::MaybePendingStateUpdate::Update(_));

    // -----------------------------------------------------------------------
    // Get a block before the forked block
    // https://sepolia.voyager.online/block/0x42dc67af5003d212ac6cd784e72db945ea4d619898f30f422358ff215cbe1e4

    let hash = felt!("0x42dc67af5003d212ac6cd784e72db945ea4d619898f30f422358ff215cbe1e4"); // 268466
    let id = BlockIdOrTag::Hash(hash);

    let block = provider.get_block_with_txs(id).await.unwrap();
    assert_matches!(block, MaybePreConfirmedBlock::Confirmed(b) if b.block_hash == hash);

    let block = provider.get_block_with_receipts(id).await.unwrap();
    assert_matches!(block, GetBlockWithReceiptsResponse::Block(b) if b.block_hash == hash);

    let block = provider.get_block_with_tx_hashes(id).await.unwrap();
    assert_matches!(block, GetBlockWithTxHashesResponse::Block(b) if b.block_hash == hash);

    let result = provider.get_block_transaction_count(id).await;
    assert!(result.is_ok());

    // TODO: uncomment this once we include genesis forked state update
    // let state = provider.get_state_update(id).await.unwrap();
    // assert_matches!(state, starknet::core::types::MaybePendingStateUpdate::Update(_));

    // -----------------------------------------------------------------------
    // Get a block that is locally generated

    for ((_, hash), _) in local_only_block {
        let id = BlockIdOrTag::Hash(hash);

        let block = provider.get_block_with_txs(id).await.unwrap();
        assert_matches!(block, MaybePreConfirmedBlock::Confirmed(b) if b.block_hash == hash);

        let block = provider.get_block_with_receipts(id).await.unwrap();
        assert_matches!(block, GetBlockWithReceiptsResponse::Block(b) if b.block_hash == hash);

        let block = provider.get_block_with_tx_hashes(id).await.unwrap();
        assert_matches!(block, GetBlockWithTxHashesResponse::Block(b) if b.block_hash == hash);

        let result = provider.get_block_transaction_count(id).await;
        assert!(result.is_ok());

        // TODO: uncomment this once we include genesis forked state update
        // let state = provider.get_state_update(id).await.unwrap();
        // assert_matches!(state, starknet::core::types::MaybePendingStateUpdate::Update(_));
    }

    // -----------------------------------------------------------------------
    // Get a block that only exist in the forked chain

    // https://sepolia.voyager.online/block/0x347a9fa25700e7a2d8f26b39c0ecf765be9a78c559b9cae722a659f25182d10
    // We only created 10 local blocks so this is fine.
    let id = BlockIdOrTag::Number(270_328);
    let result = provider.get_block_with_txs(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_with_receipts(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_with_tx_hashes(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_transaction_count(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_state_update(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    // -----------------------------------------------------------------------
    // Get block that doesn't exist on the both the forked and local chain

    let id = BlockIdOrTag::Number(i64::MAX as u64);
    let result = provider.get_block_with_txs(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_with_receipts(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_with_tx_hashes(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_block_transaction_count(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);

    let result = provider.get_state_update(id).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_transactions() -> Result<()> {
    let (_sequencer, provider, local_only_data) = setup_test().await;

    // -----------------------------------------------------------------------
    // Get txs before the forked block.

    // https://sepolia.voyager.online/tx/0x81207d4244596678e186f6ab9c833fe40a4b35291e8a90b9a163f7f643df9f
    // Transaction in block num FORK_BLOCK_NUMBER - 1
    let tx_hash = felt!("0x81207d4244596678e186f6ab9c833fe40a4b35291e8a90b9a163f7f643df9f");

    let tx = provider.get_transaction_by_hash(tx_hash).await?;
    assert_eq!(tx.transaction_hash, tx_hash);

    let tx = provider.get_transaction_receipt(tx_hash).await?;
    assert_eq!(tx.transaction_hash, tx_hash);

    let result = provider.get_transaction_status(tx_hash).await;
    assert!(result.is_ok());

    // https://sepolia.voyager.online/tx/0x1b18d62544d4ef749befadabcec019d83218d3905abd321b4c1b1fc948d5710
    // Transaction in block num FORK_BLOCK_NUMBER - 2
    let tx_hash = felt!("0x1b18d62544d4ef749befadabcec019d83218d3905abd321b4c1b1fc948d5710");

    let tx = provider.get_transaction_by_hash(tx_hash).await?;
    assert_eq!(tx.transaction_hash, tx_hash);

    let tx = provider.get_transaction_receipt(tx_hash).await?;
    assert_eq!(tx.transaction_hash, tx_hash);

    let result = provider.get_transaction_status(tx_hash).await;
    assert!(result.is_ok());

    // -----------------------------------------------------------------------
    // Get the locally created transactions.

    for (_, tx_hash) in local_only_data {
        let tx = provider.get_transaction_by_hash(tx_hash).await?;
        assert_eq!(tx.transaction_hash, tx_hash);

        let tx = provider.get_transaction_receipt(tx_hash).await?;
        assert_eq!(tx.transaction_hash, tx_hash);

        let result = provider.get_transaction_status(tx_hash).await;
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // Get a tx that exists in the forked chain but is included in a block past the forked block.

    // https://sepolia.voyager.online/block/0x335a605f2c91873f8f830a6e5285e704caec18503ca28c18485ea6f682eb65e
    // transaction in block num 268,474 (FORK_BLOCK_NUMBER + 3)
    let tx_hash = felt!("0x335a605f2c91873f8f830a6e5285e704caec18503ca28c18485ea6f682eb65e");
    let result = provider.get_transaction_by_hash(tx_hash).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::TxnHashNotFound);

    let result = provider.get_transaction_receipt(tx_hash).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::TxnHashNotFound);

    let result = provider.get_transaction_status(tx_hash).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::TxnHashNotFound);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[rstest::rstest]
#[case(BlockIdOrTag::Number(FORK_BLOCK_NUMBER))]
#[case(BlockIdOrTag::Hash(felt!("0x208950cfcbba73ecbda1c14e4d58d66a8d60655ea1b9dcf07c16014ae8a93cd")))]
async fn get_events_partially_from_forked(#[case] block_id: BlockIdOrTag) -> Result<()> {
    let (_sequencer, provider, _) = setup_test().await;
    let forked_provider = StarknetClient::new(SEPOLIA_URL.try_into().unwrap());

    // -----------------------------------------------------------------------
    // Fetch events partially from forked block.
    //
    // Here we want to make sure the continuation token is working as expected.

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(block_id),
        from_block: Some(block_id),
    };

    // events fetched directly from the forked chain.
    let result = forked_provider.get_events(filter.clone(), None, 5).await?;
    let events = result.events;

    // events fetched through the forked katana.
    let result = provider.get_events(filter, None, 5).await?;
    let forked_events = result.events;

    let token = MaybeForkedContinuationToken::parse(&result.continuation_token.unwrap())?;
    assert_matches!(token, MaybeForkedContinuationToken::Token(_));

    for (a, b) in events.iter().zip(forked_events) {
        assert_eq!(a.block_number, Some(FORK_BLOCK_NUMBER));
        assert_eq!(a.block_hash, Some(FORK_BLOCK_HASH));
        assert_eq!(a.block_number, b.block_number);
        assert_eq!(a.block_hash, b.block_hash);
        assert_eq!(a.transaction_hash, b.transaction_hash);
        assert_eq!(a.from_address, b.from_address);
        assert_eq!(a.keys, b.keys);
        assert_eq!(a.data, b.data);
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[rstest::rstest]
#[case(BlockIdOrTag::Number(FORK_BLOCK_NUMBER))]
#[case(BlockIdOrTag::Hash(felt!("0x208950cfcbba73ecbda1c14e4d58d66a8d60655ea1b9dcf07c16014ae8a93cd")))]
async fn get_events_all_from_forked(#[case] block_id: BlockIdOrTag) {
    let (_sequencer, provider, _) = setup_test().await;
    let forked_provider = StarknetClient::new(SEPOLIA_URL.try_into().unwrap());

    // -----------------------------------------------------------------------
    // Fetch events from the forked block (ie `FORK_BLOCK_NUMBER`) only.
    //
    // Based on https://sepolia.voyager.online/block/0x208950cfcbba73ecbda1c14e4d58d66a8d60655ea1b9dcf07c16014ae8a93cd, there are only 89 events in the `FORK_BLOCK_NUMBER` block.
    // So we set the chunk size to 100 to ensure we get all the events in one request.

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(block_id),
        from_block: Some(block_id),
    };

    // events fetched directly from the forked chain.
    let result = forked_provider.get_events(filter.clone(), None, 100).await.unwrap();
    let events = result.events;

    // events fetched through the forked katana.
    let result = provider.get_events(filter, None, 100).await.unwrap();
    let forked_events = result.events;

    assert!(result.continuation_token.is_none());

    for (a, b) in events.iter().zip(forked_events) {
        assert_eq!(a.block_number, Some(FORK_BLOCK_NUMBER));
        assert_eq!(a.block_hash, Some(FORK_BLOCK_HASH));
        assert_eq!(a.block_number, b.block_number);
        assert_eq!(a.block_hash, b.block_hash);
        assert_eq!(a.transaction_hash, b.transaction_hash);
        assert_eq!(a.from_address, b.from_address);
        assert_eq!(a.keys, b.keys);
        assert_eq!(a.data, b.data);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_events_local() {
    let (_sequencer, provider, local_only_data) = setup_test().await;

    // -----------------------------------------------------------------------
    // Get events from the local chain block.

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: None,
        from_block: Some(BlockIdOrTag::Number(FORK_BLOCK_NUMBER + 1)),
    };

    let result = provider.get_events(filter, None, 10).await.unwrap();
    let forked_events = result.events;

    // compare the events

    for (event, (block, tx)) in forked_events.iter().zip(local_only_data.iter()) {
        let (block_number, block_hash) = block;

        assert_eq!(event.transaction_hash, *tx);
        assert_eq!(event.block_hash, Some(*block_hash));
        assert_eq!(event.block_number, Some(*block_number));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_events_pending_exhaustive() {
    let (_sequencer, provider, local_only_data) = setup_test_pending().await;

    // -----------------------------------------------------------------------
    // Get events from the local chain pending block.

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(BlockIdOrTag::PreConfirmed),
        from_block: Some(BlockIdOrTag::PreConfirmed),
    };

    let result = provider.get_events(filter, None, 10).await.unwrap();
    let events = result.events;

    // This is expected behaviour, as the pending block is not yet closed.
    // so there may still more events to come.
    assert!(result.continuation_token.is_some());

    for (event, (_, tx)) in events.iter().zip(local_only_data.iter()) {
        assert_eq!(event.transaction_hash, *tx);
        // pending events should not have block number and block hash.
        assert_eq!(event.block_hash, None);
        assert_eq!(event.block_number, None);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[rstest::rstest]
#[case(BlockIdOrTag::Number(FORK_BLOCK_NUMBER))]
#[case(BlockIdOrTag::Hash(felt!("0x208950cfcbba73ecbda1c14e4d58d66a8d60655ea1b9dcf07c16014ae8a93cd")))] // FORK_BLOCK_NUMBER hash
async fn get_events_forked_and_local_boundary_exhaustive(#[case] block_id: BlockIdOrTag) {
    let (_sequencer, provider, local_only_data) = setup_test().await;
    let forked_provider = StarknetClient::new(SEPOLIA_URL.try_into().unwrap());

    // -----------------------------------------------------------------------
    // Get events from that cross the boundaries between forked and local chain block.
    //
    // Total events in `FORK_BLOCK_NUMBER` block is 89. While `FORK_BLOCK_NUMBER` + 1 is 1 ∴ 89 + 1
    // = 90 events.

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(block_id),
        from_block: Some(block_id),
    };

    // events fetched directly from the forked chain.
    let result = forked_provider.get_events(filter.clone(), None, 100).await.unwrap();
    let events = result.events;

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(BlockIdOrTag::Latest),
        from_block: Some(block_id),
    };

    let result = provider.get_events(filter, None, 100).await.unwrap();
    let boundary_events = result.events;

    // because we're pointing to latest block, we should not have anymore continuation token.
    assert!(result.continuation_token.is_none());

    let forked_events = &boundary_events[..89];
    let local_events = &boundary_events[89..];

    similar_asserts::assert_eq!(forked_events, events);

    for (event, (block, tx)) in local_events.iter().zip(local_only_data.iter()) {
        let (block_number, block_hash) = block;

        assert_eq!(event.transaction_hash, *tx);
        assert_eq!(event.block_number, Some(*block_number));
        assert_eq!(event.block_hash, Some(*block_hash));
    }
}

#[tokio::test(flavor = "multi_thread")]
#[rstest::rstest]
#[case(BlockIdOrTag::Number(FORK_BLOCK_NUMBER - 1))]
#[case(BlockIdOrTag::Hash(felt!("0x4a6a79bfefceb03af4f78758785b0c40ddf9f757e9a8f72f01ecb0aad11e298")))] // FORK_BLOCK_NUMBER - 1 hash
async fn get_events_forked_and_local_boundary_non_exhaustive(#[case] block_id: BlockIdOrTag) {
    let (_sequencer, provider, _) = setup_test().await;
    let forked_provider = StarknetClient::new(SEPOLIA_URL.try_into().unwrap());

    // -----------------------------------------------------------------------
    // Get events that cross the boundaries between forked and local chain block, but
    // not all events from the forked range is fetched.

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(block_id),
        from_block: Some(block_id),
    };

    // events fetched directly from the forked chain.
    let result = forked_provider.get_events(filter.clone(), None, 50).await.unwrap();
    let forked_events = result.events;

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(BlockIdOrTag::PreConfirmed),
        from_block: Some(block_id),
    };

    let result = provider.get_events(filter, None, 50).await.unwrap();
    let katana_events = result.events;

    let token = MaybeForkedContinuationToken::parse(&result.continuation_token.unwrap()).unwrap();
    assert_matches!(token, MaybeForkedContinuationToken::Token(_));
    similar_asserts::assert_eq!(katana_events, forked_events);
}

#[tokio::test(flavor = "multi_thread")]
#[rstest::rstest]
#[case::doesnt_exist_at_all(felt!("0x123"))]
#[case::after_forked_block_but_on_the_forked_chain(felt!("0x21f4c20f9cc721dbaee2eaf44c79342b37c60f55ac37c13a4bdd6785ac2a5e5"))]
async fn get_events_with_invalid_block_hash(#[case] hash: BlockHash) {
    let (_sequencer, provider, _) = setup_test().await;

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(BlockIdOrTag::Hash(hash)),
        from_block: Some(BlockIdOrTag::Hash(hash)),
    };

    let result = provider.get_events(filter.clone(), None, 5).await.unwrap_err();
    assert_provider_starknet_err!(result, StarknetApiError::BlockNotFound);
}

#[cfg(test)]
mod tests {
    use katana_core::service::block_producer::IntervalBlockProducer;
    use katana_db::Db;
    use katana_primitives::class::ClassHash;
    use katana_primitives::state::StateUpdates;
    use katana_primitives::ContractAddress;
    use katana_primitives::Felt;
    use katana_provider::api::block::BlockNumberProvider;
    use katana_provider::api::trie::TrieWriter;
    use katana_provider::MutableProvider;
    use katana_provider::{ForkProviderFactory, ProviderFactory};
    use katana_utils::TestNode;
    use proptest::arbitrary::any;
    use proptest::prelude::{Just, ProptestConfig, Strategy};
    use proptest::prop_assert_eq;
    use proptest::proptest;
    use rand::{thread_rng, Rng};
    use std::collections::{BTreeMap, BTreeSet};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_commit_new_state_root_mainnet_blockchain_and_forked_provider() {
        use katana_utils::TestNode;

        let sequencer = TestNode::new().await;
        let backend = sequencer.backend();
        let provider = backend.storage.provider();
        let provider_mut = backend.storage.provider_mut();

        let block_number = provider.latest_number().unwrap();

        let state_updates = setup_mainnet_updates_randomized(5);

        provider_mut.compute_state_root(block_number, &state_updates).unwrap();
        provider_mut.commit().unwrap();

        let fork_minimal_updates = setup_mainnet_updates_randomized(5);

        let db = Db::in_memory().unwrap();
        let starknet_rpc_client = sequencer.starknet_rpc_client();
        let fork_factory = ForkProviderFactory::new(db, block_number, starknet_rpc_client);

        let state_root = {
            let forked_provider = fork_factory.provider_mut();
            let root =
                forked_provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();
            forked_provider.commit().unwrap();
            root
        };

        let provider_mut = backend.storage.provider_mut();
        let mainnet_state_root_same_updates =
            provider_mut.compute_state_root(block_number, &fork_minimal_updates).unwrap();
        provider_mut.commit().unwrap();

        assert_eq!(
            state_root, mainnet_state_root_same_updates,
            "State roots do not match on first run: fork={:?}, mainnet={:?}",
            state_root, mainnet_state_root_same_updates
        );

        // Second iteration with new random updates
        let state_updates = setup_mainnet_updates_randomized(5);
        //IT's important here to compute state root for forked network first, then for mainnet
        //otherwise it will be different roots because it's like double computation of same changes
        let fork_state_root = {
            let forked_provider = fork_factory.provider_mut();
            let root = forked_provider.compute_state_root(block_number, &state_updates).unwrap();
            forked_provider.commit().unwrap();
            root
        };
        let provider_mut = backend.storage.provider_mut();
        let mainnet_state_root =
            provider_mut.compute_state_root(block_number, &state_updates).unwrap();
        provider_mut.commit().unwrap();

        assert_eq!(
            mainnet_state_root, fork_state_root,
            "State roots do not match on second run: fork={:?}, mainnet={:?}",
            fork_state_root, mainnet_state_root
        );
    }

    fn setup_mainnet_updates_randomized(num_contracts: usize) -> StateUpdates {
        let mut state_updates = StateUpdates::default();

        for _ in 0..num_contracts {
            let (address, class_hash, storage, nonce) = random_contract();
            state_updates.deployed_contracts.insert(address, class_hash);
            state_updates.storage_updates.insert(address, storage);
            state_updates.declared_classes.insert(class_hash, random_felt());
            state_updates.nonce_updates.insert(address, nonce);
            if thread_rng().gen_bool(0.2) {
                let new_class_hash = random_class_hash();
                state_updates.replaced_classes.insert(address, new_class_hash);
                state_updates.declared_classes.insert(new_class_hash, random_felt());
            }
            if thread_rng().gen_bool(0.2) {
                state_updates.deprecated_declared_classes.insert(random_class_hash());
            }
        }

        state_updates
    }

    fn random_felt() -> Felt {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill(&mut bytes);
        Felt::from_bytes_be(&bytes)
    }

    fn random_class_hash() -> ClassHash {
        ClassHash::from(random_felt())
    }

    fn random_contract_address() -> ContractAddress {
        ContractAddress::from(random_felt())
    }

    fn random_contract() -> (ContractAddress, ClassHash, BTreeMap<Felt, Felt>, Felt) {
        let address = random_contract_address();
        let class_hash = random_class_hash();
        let mut storage = BTreeMap::new();
        for _ in 0..thread_rng().gen_range(1..=3) {
            storage.insert(random_felt(), random_felt());
        }
        let nonce = random_felt();
        (address, class_hash, storage, nonce)
    }

    fn arb_felt() -> impl Strategy<Value = Felt> {
        any::<[u8; 32]>().prop_map(|bytes| Felt::from_bytes_be(&bytes))
    }

    fn arb_class_hash() -> impl Strategy<Value = ClassHash> {
        arb_felt().prop_map(ClassHash::from)
    }

    fn arb_contract_address() -> impl Strategy<Value = ContractAddress> {
        arb_felt().prop_map(ContractAddress::from)
    }

    fn arb_storage() -> impl Strategy<Value = BTreeMap<Felt, Felt>> {
        proptest::collection::btree_map(arb_felt(), arb_felt(), 0..3)
    }

    fn arb_state_updates() -> impl Strategy<Value = StateUpdates> {
        proptest::collection::btree_map(
            arb_contract_address(),
            (arb_class_hash(), arb_storage(), arb_felt()),
            1..6,
        )
        .prop_flat_map(|contracts| {
            // Rozbij na odpowiednie pola
            let mut deployed_contracts = BTreeMap::new();
            let mut storage_updates = BTreeMap::new();
            let mut nonce_updates = BTreeMap::new();
            let mut declared_classes = BTreeMap::new();
            let replaced_classes = BTreeMap::new();
            let deprecated_declared_classes = BTreeSet::new();

            for (address, (class_hash, storage, nonce)) in &contracts {
                deployed_contracts.insert(*address, *class_hash);
                storage_updates.insert(*address, storage.clone());
                nonce_updates.insert(*address, *nonce);
                declared_classes.insert(*class_hash, Felt::from(1u8));
            }

            Just(StateUpdates {
                deployed_contracts,
                storage_updates,
                nonce_updates,
                declared_classes,
                replaced_classes,
                deprecated_declared_classes,
                ..Default::default()
            })
        })
    }

    // These tests require workaround to work
    // Comment out "let global_class_cache = class_cache.build_global()?;"
    // in Node::build()
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 50,
            .. ProptestConfig::default()
        })]
        #[test]
        fn prop_state_roots_match_for_mainnet_and_forked(
            num_iters in 1usize..=5,
            state_updates_vec in proptest::collection::vec(arb_state_updates(), 1..=5),
            fork_minimal_updates_vec in proptest::collection::vec(arb_state_updates(), 1..=5)
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let _ = rt.block_on(async {

                let sequencer = TestNode::new().await;
                let backend = sequencer.backend();
                let provider = backend.storage.provider();
                let mut block_number = provider.latest_number().unwrap();
                let mut producer = IntervalBlockProducer::new(backend.clone(), None);

                let initial_state = &state_updates_vec[0];
                let provider_mut = backend.storage.provider_mut();
                provider_mut.compute_state_root(block_number, initial_state).unwrap();
                provider_mut.commit().unwrap();
                producer.force_mine();
                block_number = provider.latest_number().unwrap();

                let db = Db::in_memory().unwrap();
                let starknet_rpc_client = sequencer.starknet_rpc_client();
                let fork_factory = ForkProviderFactory::new(db, block_number, starknet_rpc_client);

                for i in 0..num_iters {
                    let fork_minimal_updates = &fork_minimal_updates_vec[i % fork_minimal_updates_vec.len()];

                    let fork_root = {
                        let forked_provider = fork_factory.provider_mut();
                        let root = forked_provider.compute_state_root(block_number, fork_minimal_updates).unwrap();
                        forked_provider.commit().unwrap();
                        root
                    };
                    let provider_mut = backend.storage.provider_mut();
                    let mainnet_root = provider_mut.compute_state_root(block_number, fork_minimal_updates).unwrap();
                    provider_mut.commit().unwrap();

                    prop_assert_eq!(fork_root, mainnet_root, "State roots do not match at iteration {}", i);

                    producer.force_mine();
                    block_number = provider.latest_number().unwrap();
                }
                Ok(())
            });
        }
    }
}
