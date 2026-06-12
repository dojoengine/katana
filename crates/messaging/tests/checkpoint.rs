//! Live-rewind checkpoint tests for the messaging service against an Ethereum
//! settlement layer.
//!
//! These drive checkpoint changes through [`MessagingController`] (NOT a
//! JSON-RPC client — `katana-messaging` sits below the node/RPC layer). They
//! exercise the operator-rewind path end-to-end against a real Anvil node and
//! the real `StarknetMessagingLocal` Solidity contract: `set_checkpoint` /
//! `reset_checkpoint` cause the running drain task to re-gather, and the pool's
//! hash-level dedup must absorb the second pass without duplicating L2 txs.

use std::time::{Duration, Instant};

use alloy_primitives::{Uint, U256};
use alloy_provider::ProviderBuilder;
use alloy_sol_types::sol;
use katana_messaging::{MessagingService, SettlementChainConfig};
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::TxHash;
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

/// Poll the provider's messaging checkpoint until it is `Some` with
/// `block >= min_block`, or `timeout` elapses.
async fn wait_for_checkpoint_at_or_above(
    provider: &katana_provider::DbProviderFactory,
    min_block: u64,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(cp) = common::messaging_checkpoint(provider) {
            if cp.block >= min_block {
                return;
            }
        }
        if Instant::now() >= deadline {
            panic!("checkpoint did not advance to >= {min_block} within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Poll the L1->L2 index until `l1_hash` maps to a non-empty list, then return it.
async fn wait_for_l1_to_l2_mapping(
    provider: &katana_provider::DbProviderFactory,
    l1_hash: &[u8; 32],
    timeout: Duration,
) -> Vec<TxHash> {
    let deadline = Instant::now() + timeout;
    loop {
        let mapped = common::l2_txs_for_l1(provider, l1_hash);
        if !mapped.is_empty() {
            return mapped;
        }
        if Instant::now() >= deadline {
            panic!("L1->L2 mapping never recorded within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Mid-flight set: after the service has processed a message and advanced the
/// checkpoint, setting the checkpoint to a *prior* block must cause a
/// re-gather. The pool's hash-level dedup must prevent duplicate L2 txs and the
/// checkpoint must re-advance to (at least) its prior value.
#[tokio::test(flavor = "multi_thread")]
async fn set_checkpoint_mid_flight_causes_re_gather() {
    let port: u16 = rand::thread_rng().gen_range(35000..65000);

    let l1_provider = ProviderBuilder::new()
        .connect_anvil_with_wallet_and_config(|anvil| anvil.port(port))
        .expect("failed to build eth provider");

    let core_contract = StarknetContract::deploy(&l1_provider).await.unwrap();
    let l1_test = Contract1::deploy(&l1_provider, *core_contract.address()).await.unwrap();

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

    // Grab the controller BEFORE `start()` takes the rewind receiver — we need
    // it to drive a live rewind while the service runs.
    let controller = service.controller();
    let mut handle = service.start().expect("start messaging service");

    // Send one L1->L2 message.
    let recipient = ContractAddress::from(felt!("0xbeef"));
    let entry_point_selector = selector!("msg_handler_value");
    let calldata = [123u8];
    let receipt = l1_test
        .sendMessage(
            recipient.into(),
            U256::from_be_bytes(entry_point_selector.to_bytes_be()),
            calldata.iter().map(|x| U256::from(*x)).collect::<Vec<_>>(),
        )
        .gas(12_000_000)
        .value(Uint::from(1))
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "L1 sendMessage tx reverted");
    let l1_hash: [u8; 32] = receipt.transaction_hash.0;

    // Wait for the service to process it.
    let mapped_before =
        wait_for_l1_to_l2_mapping(&provider, &l1_hash, Duration::from_secs(15)).await;
    assert_eq!(mapped_before.len(), 1, "single L1 tx -> single L2 tx");

    let cp_before =
        common::messaging_checkpoint(&provider).expect("checkpoint should have advanced");
    assert!(cp_before.block > 0, "expected checkpoint to have advanced");

    // Operator action: rewind below the current checkpoint to force a re-gather.
    controller.set_checkpoint(0, 0).await.expect("set_checkpoint succeeds");
    wait_for_checkpoint_at_or_above(&provider, cp_before.block, Duration::from_secs(15)).await;

    // The L1->L2 index must still hold exactly one L2 tx for this L1 hash. The
    // re-gather re-published the L1Handler to the pool; the pool's hash-level
    // dedup absorbed the duplicate.
    let mapped_after = common::l2_txs_for_l1(&provider, &l1_hash);
    assert_eq!(
        mapped_after.len(),
        1,
        "re-gather must not duplicate L1->L2 mapping (pool dedup contract)"
    );
    assert_eq!(mapped_after, mapped_before, "same L2 tx hash both times");

    handle.stop();
    handle.stopped().await;
}

/// Reset clears the persisted checkpoint and rewinds the service to the
/// configured `from_block`. `get_checkpoint` must return `None` immediately
/// after reset, and a fresh L1 message must still be processed — the service
/// keeps running and re-gathers cleanly.
#[tokio::test(flavor = "multi_thread")]
async fn reset_checkpoint_resumes_from_configured_from_block() {
    let port: u16 = rand::thread_rng().gen_range(35000..65000);

    let l1_provider = ProviderBuilder::new()
        .connect_anvil_with_wallet_and_config(|anvil| anvil.port(port))
        .expect("failed to build eth provider");

    let core_contract = StarknetContract::deploy(&l1_provider).await.unwrap();
    let l1_test = Contract1::deploy(&l1_provider, *core_contract.address()).await.unwrap();

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

    let controller = service.controller();
    let mut handle = service.start().expect("start messaging service");

    let recipient = ContractAddress::from(felt!("0xbeef"));
    let entry_point_selector = selector!("msg_handler_value");

    // Send first message and wait for the checkpoint to record it.
    let receipt = l1_test
        .sendMessage(
            recipient.into(),
            U256::from_be_bytes(entry_point_selector.to_bytes_be()),
            vec![U256::from(7u8)],
        )
        .gas(12_000_000)
        .value(Uint::from(1))
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status());
    wait_for_checkpoint_at_or_above(&provider, 1, Duration::from_secs(15)).await;

    // Reset clears the row. `get_checkpoint` must observe `None` right away —
    // the DB delete is synchronous (the live rewind signal is fire-and-forget).
    controller.reset_checkpoint().await.expect("reset_checkpoint succeeds");
    let cp_post_reset = controller.get_checkpoint().expect("get_checkpoint succeeds");
    assert!(cp_post_reset.is_none(), "reset deletes the row; get returns None immediately after");

    // Fresh message after reset: the service kept running and still processes it,
    // re-establishing the checkpoint from the configured `from_block`.
    let receipt = l1_test
        .sendMessage(
            recipient.into(),
            U256::from_be_bytes(entry_point_selector.to_bytes_be()),
            vec![U256::from(13u8)],
        )
        .gas(12_000_000)
        .value(Uint::from(1))
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status());
    let l1_hash_b: [u8; 32] = receipt.transaction_hash.0;
    let mapped = wait_for_l1_to_l2_mapping(&provider, &l1_hash_b, Duration::from_secs(15)).await;
    assert_eq!(mapped.len(), 1, "fresh message -> single L2 tx");

    handle.stop();
    handle.stopped().await;
}

/// Pool dedup contract: re-gather of an already-processed block must NOT add a
/// second L2 tx for the same L1 hash. Directly checks the DupSort index stays
/// single-entry through the rewind round-trip.
#[tokio::test(flavor = "multi_thread")]
async fn re_gather_after_rewind_does_not_duplicate_l2_txs() {
    let port: u16 = rand::thread_rng().gen_range(35000..65000);

    let l1_provider = ProviderBuilder::new()
        .connect_anvil_with_wallet_and_config(|anvil| anvil.port(port))
        .expect("failed to build eth provider");

    let core_contract = StarknetContract::deploy(&l1_provider).await.unwrap();
    let l1_test = Contract1::deploy(&l1_provider, *core_contract.address()).await.unwrap();

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

    let controller = service.controller();
    let mut handle = service.start().expect("start messaging service");

    // Send exactly one L1 message.
    let recipient = ContractAddress::from(felt!("0xbeef"));
    let entry_point_selector = selector!("msg_handler_value");
    let receipt = l1_test
        .sendMessage(
            recipient.into(),
            U256::from_be_bytes(entry_point_selector.to_bytes_be()),
            vec![U256::from(42u8)],
        )
        .gas(12_000_000)
        .value(Uint::from(1))
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status());
    let l1_hash: [u8; 32] = receipt.transaction_hash.0;

    // Wait for the single L2 tx to be recorded, capture it.
    let initial = wait_for_l1_to_l2_mapping(&provider, &l1_hash, Duration::from_secs(15)).await;
    assert_eq!(initial.len(), 1);
    let l2_hash = initial[0];

    // Force a re-gather by rewinding below the current checkpoint.
    let cp_before = common::messaging_checkpoint(&provider).expect("checkpoint exists");
    controller.set_checkpoint(0, 0).await.expect("set_checkpoint succeeds");
    wait_for_checkpoint_at_or_above(&provider, cp_before.block, Duration::from_secs(15)).await;

    // Still exactly one mapping, still the same L2 hash.
    let final_mapping = common::l2_txs_for_l1(&provider, &l1_hash);
    assert_eq!(final_mapping.len(), 1, "no duplicates after rewind");
    assert_eq!(final_mapping[0], l2_hash, "same L2 tx hash as before rewind");

    handle.stop();
    handle.stopped().await;
}
