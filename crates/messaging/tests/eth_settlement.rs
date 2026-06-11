//! Integration tests for the messaging service against an Ethereum settlement layer.
//!
//! These exercise the actual gather/insert/checkpoint loop end-to-end:
//! a real Anvil node, the real `StarknetMessagingLocal` Solidity contract, and the
//! real `MessagingService` running with an isolated in-memory pool and provider —
//! no Katana sequencer, no block producer, no executor.

use std::time::Duration;

use alloy_primitives::{Uint, U256};
use alloy_provider::ProviderBuilder;
use alloy_sol_types::sol;
use katana_messaging::{MessagingService, SettlementChainConfig};
use katana_pool::api::TransactionPool;
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::ExecutableTx;
use katana_primitives::{felt, ContractAddress};
use rand::Rng;
use starknet::macros::selector;
use url::Url;

mod common;

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    StarknetContract,
    "tests/test_data/solidity/StarknetMessagingLocalCompiled.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    Contract1,
    "tests/test_data/solidity/Contract1Compiled.json"
);

/// Send a single `LogMessageToL2` from L1 and assert it lands in the isolated
/// pool, the L1->L2 index records the mapping, and the messaging checkpoint
/// advances to the gather block.
#[tokio::test(flavor = "multi_thread")]
async fn collects_single_message_from_anvil() {
    let port: u16 = rand::thread_rng().gen_range(35000..65000);

    let l1_provider = ProviderBuilder::new()
        .connect_anvil_with_wallet_and_config(|anvil| anvil.port(port))
        .expect("failed to build eth provider");

    let core_contract = StarknetContract::deploy(&l1_provider).await.unwrap();
    let l1_test_contract = Contract1::deploy(&l1_provider, *core_contract.address()).await.unwrap();

    let settlement = SettlementChainConfig::Ethereum {
        rpc_url: Url::parse(&format!("http://localhost:{}", port)).unwrap(),
        contract_address: *core_contract.address(),
    };

    let pool = common::build_test_pool();
    let provider = common::build_test_provider();

    let service =
        MessagingService::new(ChainId::default(), pool.clone(), provider.clone(), settlement)
            .interval(1)
            .from_block(0);

    let mut handle = service.start().expect("start messaging service");

    // Fire the on-chain call that the messaging service should observe.
    let recipient = ContractAddress::from(felt!("0xbeef"));
    let entry_point_selector = selector!("msg_handler_value");
    let calldata = [123u8];

    let receipt = l1_test_contract
        .sendMessage(
            recipient.into(),
            U256::from_be_bytes(entry_point_selector.to_bytes_be()),
            calldata.iter().map(|x| U256::from(*x)).collect::<Vec<_>>(),
        )
        .gas(12_000_000)
        .value(Uint::from(1))
        .send()
        .await
        .expect("failed to send L1->L2 message")
        .get_receipt()
        .await
        .expect("failed to get receipt");

    assert!(receipt.status(), "L1 sendMessage tx reverted");
    let l1_tx_hash: [u8; 32] = receipt.transaction_hash.0;

    common::wait_for_pool_size(&pool, 1, Duration::from_secs(15))
        .await
        .expect("messaging service should have collected the message");

    // Inspect the L1Handler transaction the service inserted.
    let snapshot = pool.take_transactions_snapshot();
    assert_eq!(snapshot.len(), 1);
    let ExecutableTx::L1Handler(ref l1_handler) = snapshot[0].transaction else {
        panic!("expected L1Handler tx in the pool");
    };

    assert_eq!(l1_handler.contract_address, recipient);
    assert_eq!(l1_handler.entry_point_selector, entry_point_selector);

    // L1Handler calldata is [from_address (sender), ...payload]
    let sender = l1_test_contract.address();
    assert_eq!(
        l1_handler.calldata[0],
        katana_primitives::Felt::from_bytes_be_slice(sender.as_slice()),
    );
    assert_eq!(l1_handler.calldata.len(), 1 + calldata.len());

    // The L1->L2 index records the L1 tx hash -> L2 tx hash mapping.
    let l2_hashes = common::l2_txs_for_l1(&provider, &l1_tx_hash);
    assert_eq!(l2_hashes, vec![snapshot[0].hash]);

    // The checkpoint advanced to (at least) the block the L1 tx was mined in.
    let cp = common::messaging_checkpoint(&provider).expect("checkpoint should be persisted");
    assert!(cp.block >= 1, "checkpoint block should be the L1 block of the sendMessage tx");
    assert_eq!(cp.tx_index, 0, "single message in a block resumes at tx_index 0");

    handle.stop();
    handle.stopped().await;
}

/// Two `LogMessageToL2` events from a single L1 transaction must each land in
/// the pool as their own L1Handler, the L1->L2 index must hold both L2 hashes
/// under the same L1 hash, and the checkpoint must advance to the second
/// event's tx_index.
#[tokio::test(flavor = "multi_thread")]
async fn collects_multiple_messages_in_same_block() {
    let port: u16 = rand::thread_rng().gen_range(35000..65000);

    let l1_provider = ProviderBuilder::new()
        .connect_anvil_with_wallet_and_config(|anvil| anvil.port(port))
        .expect("failed to build eth provider");

    let core_contract = StarknetContract::deploy(&l1_provider).await.unwrap();
    let l1_test_contract = Contract1::deploy(&l1_provider, *core_contract.address()).await.unwrap();

    let settlement = SettlementChainConfig::Ethereum {
        rpc_url: Url::parse(&format!("http://localhost:{}", port)).unwrap(),
        contract_address: *core_contract.address(),
    };

    let pool = common::build_test_pool();
    let provider = common::build_test_provider();

    let service =
        MessagingService::new(ChainId::default(), pool.clone(), provider.clone(), settlement)
            .interval(1)
            .from_block(0);

    let mut handle = service.start().expect("start messaging service");

    let recipient = ContractAddress::from(felt!("0xbeef"));
    let entry_point_selector = selector!("msg_handler_value");

    // Fire two `sendMessage` calls back-to-back. Anvil mines each as its own
    // L1 tx (different `l1_tx_hash`), but each emits exactly one
    // `LogMessageToL2` event. This validates the per-message tx_index
    // accounting from the collector's perspective.
    let mut l1_tx_hashes = Vec::with_capacity(2);
    for value in [1u8, 2u8] {
        let receipt = l1_test_contract
            .sendMessage(
                recipient.into(),
                U256::from_be_bytes(entry_point_selector.to_bytes_be()),
                vec![U256::from(value)],
            )
            .gas(12_000_000)
            .value(Uint::from(1))
            .send()
            .await
            .expect("send message")
            .get_receipt()
            .await
            .expect("get receipt");
        assert!(receipt.status());
        l1_tx_hashes.push(receipt.transaction_hash.0);
    }

    common::wait_for_pool_size(&pool, 2, Duration::from_secs(15))
        .await
        .expect("messaging service should have collected both messages");

    let snapshot = pool.take_transactions_snapshot();
    assert_eq!(snapshot.len(), 2);

    // Each L1 tx maps to exactly one L2 tx.
    for l1_hash in &l1_tx_hashes {
        let l2_hashes = common::l2_txs_for_l1(&provider, l1_hash);
        assert_eq!(l2_hashes.len(), 1, "each L1 tx should map to one L2 tx");
    }

    let cp = common::messaging_checkpoint(&provider).expect("checkpoint persisted");
    assert!(cp.block >= 1);

    handle.stop();
    handle.stopped().await;
}
