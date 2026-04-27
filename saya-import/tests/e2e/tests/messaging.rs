//! E2E messaging tests — L2↔L3 message passing via Piltover.
//!
//! The tests manage the Docker Compose lifecycle themselves.
//! Run them serially to avoid lifecycle conflicts:
//!
//!   cargo test --test messaging -p saya-e2e -- --nocapture --test-threads=1

use saya_e2e::{
    compose_up, env, provider, wait_for_l1_handler, wait_for_settlement, wait_for_tx_block,
    ComposeGuard,
};
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, Call, ExecutionResult, Felt, TransactionReceipt};
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::macros::selector;
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use std::time::Duration;

fn build_account(
    rpc: JsonRpcClient<HttpTransport>,
    private_key: Felt,
    account_address: Felt,
    chain_id_str: &str,
) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(private_key));
    let chain_id = cairo_short_string_to_felt(chain_id_str).expect("invalid chain id string");
    let mut account = SingleOwnerAccount::new(
        rpc,
        signer,
        account_address,
        chain_id,
        ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::Latest));
    account
}

/// L2 → L3: send a message from `sn_msg` on L2 and confirm the L1Handler fires on L3.
///
/// The message is near-instant: Katana L3 picks up L1→L2 messages automatically
/// without waiting for Saya settlement.
#[tokio::test]
async fn test_l2_to_l3_message() {
    compose_up();
    let _guard = ComposeGuard;

    let l3_provider = provider(&env::l3_rpc_url());

    // Record the current L3 tip before we send so we don't miss the tx.
    let start_block = l3_provider
        .block_number()
        .await
        .expect("failed to get L3 block number");

    let l2_account = build_account(
        provider(&env::settlement_rpc_url()),
        env::l2_private_key(),
        env::l2_account_address(),
        "KATANA",
    );

    // invoke sn_msg.send_message(to_address, selector, value=888)
    l2_account
        .execute_v3(vec![Call {
            to: env::sn_msg_address(),
            selector: selector!("send_message"),
            calldata: vec![
                env::appc_msg_sn_address(),
                selector!("msg_handler_value"),
                Felt::from(888u64),
            ],
        }])
        .send()
        .await
        .expect("failed to send L2→L3 message");

    // Wait for the L1Handler tx to appear on L3.
    let l1h_hash = wait_for_l1_handler(
        &l3_provider,
        selector!("msg_handler_value"),
        start_block,
        Duration::from_secs(60),
        Duration::from_secs(2),
    )
    .await
    .expect("L1Handler tx did not appear on L3 within timeout");

    // Verify it succeeded.
    let receipt = l3_provider
        .get_transaction_receipt(l1h_hash)
        .await
        .expect("failed to get L1Handler receipt");

    let execution_result = match receipt.receipt {
        TransactionReceipt::L1Handler(r) => r.execution_result,
        other => panic!("expected L1Handler receipt, got {other:?}"),
    };
    assert!(
        matches!(execution_result, ExecutionResult::Succeeded),
        "L1Handler tx reverted: {execution_result:?}"
    );
}

/// L3 → L2: send a message from `appc_msg_sn` on L3, wait for Saya to settle
/// the block, then consume the message on L2 via `sn_msg.consume_message_value`.
#[tokio::test]
async fn test_l3_to_l2_message() {
    compose_up();
    let _guard = ComposeGuard;

    let l2_provider = provider(&env::settlement_rpc_url());
    let l3_provider = provider(&env::l3_rpc_url());

    let l3_account = build_account(
        provider(&env::l3_rpc_url()),
        env::appc_private_key(),
        env::appc_account_address(),
        "custom",
    );

    // invoke appc_msg_sn.send_message(to_address, value=111)
    let send_result = l3_account
        .execute_v3(vec![Call {
            to: env::appc_msg_sn_address(),
            selector: selector!("send_message"),
            calldata: vec![env::sn_msg_address(), Felt::from(111u64)],
        }])
        .send()
        .await
        .expect("failed to send L3→L2 message");

    // Wait for the tx to land in a confirmed block and get that block number.
    // Using block_number() directly is racy — the tx may still be pending.
    let l3_block = wait_for_tx_block(
        &l3_provider,
        send_result.transaction_hash,
        Duration::from_secs(30),
        Duration::from_millis(500),
    )
    .await
    .expect("send_message tx was not confirmed in time");

    // Wait for Saya to settle that block on L2.
    wait_for_settlement(
        &l2_provider,
        env::piltover_address(),
        l3_block,
        Duration::from_secs(180),
        Duration::from_secs(3),
    )
    .await
    .expect("Saya did not settle L3 block within timeout");

    let l2_account = build_account(
        provider(&env::settlement_rpc_url()),
        env::l2_private_key(),
        env::l2_account_address(),
        "KATANA",
    );

    // invoke sn_msg.consume_message_value(from_address, value=111)
    // Piltover must have the message hash; if not it reverts with INVALID_MESSAGE_TO_CONSUME.
    let consume_result = l2_account
        .execute_v3(vec![Call {
            to: env::sn_msg_address(),
            selector: selector!("consume_message_value"),
            calldata: vec![env::appc_msg_sn_address(), Felt::from(111u64)],
        }])
        .send()
        .await
        .expect("failed to consume L3→L2 message");

    wait_for_tx_block(
        &l2_provider,
        consume_result.transaction_hash,
        Duration::from_secs(30),
        Duration::from_millis(500),
    )
    .await
    .expect("consume tx was not confirmed in time");

    let receipt = l2_provider
        .get_transaction_receipt(consume_result.transaction_hash)
        .await
        .expect("failed to get consume receipt");

    let execution_result = match receipt.receipt {
        TransactionReceipt::Invoke(r) => r.execution_result,
        other => panic!("expected Invoke receipt, got {other:?}"),
    };
    assert!(
        matches!(execution_result, ExecutionResult::Succeeded),
        "consume_message_value reverted: {execution_result:?}"
    );
}
