//! End-to-end test for resetting a piltover settlement contract to genesis.
//!
//! Uses a Katana instance AS the settlement (L1) chain, deploys the real piltover appchain class
//! via the same `deployment::deploy_settlement_contract` the `katana init rollup` flow uses, drives
//! the Starknet → Appchain message nonce up by sending messages, then calls
//! `deployment::reset_settlement_contract` (the `--force` path's worker) and asserts the contract
//! is back in its freshly-deployed state:
//!
//! - the state checkpoint reads back as genesis `(0, Felt::MAX, 0)`, and
//! - the next message restarts at nonce `0`.

use katana::cli::init::deployment::{deploy_settlement_contract, reset_settlement_contract};
use katana_chain_spec::settlement_check::SettlementChainProvider;
use katana_primitives::ContractAddress;
use katana_utils::{TestNode, TxWaiter};
use piltover::AppchainContractReader;
use starknet::accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, Call, Felt, TransactionReceipt};
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::macros::selector;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use url::Url;

/// A throwaway fact-registry address. In StarknetOs mode the value is only stored and echoed back
/// by `check_program_info`, so any felt works for the test.
const FACT_REGISTRY: Felt = Felt::from_hex_unchecked("0x1");

type SettlementAccount = SingleOwnerAccount<SettlementChainProvider, LocalWallet>;
type L1Account = SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>;

/// Builds an owner account over a [`SettlementChainProvider`] (the account type the init
/// `deployment` helpers require), signing with the settlement node's first prefunded dev account.
fn settlement_account(node: &TestNode) -> SettlementAccount {
    let url = Url::parse(&format!("http://{}", node.rpc_addr())).expect("settlement rpc url");
    let provider = SettlementChainProvider::new(url, FACT_REGISTRY);

    let (address, account) =
        node.backend().chain_spec.genesis().accounts().next().expect("a prefunded dev account");
    let private_key = account.private_key().expect("dev account has a private key");
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(private_key));

    let mut account = SingleOwnerAccount::new(
        provider,
        signer,
        (*address).into(),
        node.backend().chain_spec.id().into(),
        ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    account
}

/// Sends one `send_message_to_appchain(...)` and returns the nonce carried by the emitted
/// `MessageSent` event. The event data layout is `[selector, nonce, payload_len, ...payload]`, so
/// the nonce is at index 1.
async fn send_message_nonce(
    node: &TestNode,
    account: &L1Account,
    contract: ContractAddress,
) -> Felt {
    let recipient = Felt::from(0xbeef_u64);
    let entry_point = selector!("handle_message");
    let payload = vec![Felt::from(1_u64)];

    let mut calldata = vec![recipient, entry_point, Felt::from(payload.len() as u64)];
    calldata.extend_from_slice(&payload);

    let call =
        Call { to: contract.into(), selector: selector!("send_message_to_appchain"), calldata };

    let invoke = account.execute_v3(vec![call]).send().await.expect("send_message_to_appchain");
    TxWaiter::new(invoke.transaction_hash, &node.starknet_rpc_client())
        .await
        .expect("send_message_to_appchain tx accepted");

    let receipt = account
        .provider()
        .get_transaction_receipt(invoke.transaction_hash)
        .await
        .expect("get receipt");

    let events = match receipt.receipt {
        TransactionReceipt::Invoke(r) => r.events,
        other => panic!("expected an invoke receipt, got {other:?}"),
    };

    let contract_felt: Felt = contract.into();
    let message_sent = selector!("MessageSent");
    let event = events
        .into_iter()
        .find(|e| e.from_address == contract_felt && e.keys.first() == Some(&message_sent))
        .expect("a MessageSent event from the settlement contract");

    event.data[1]
}

#[tokio::test(flavor = "multi_thread")]
async fn settlement_contract_reset_to_genesis() {
    // Run a Katana node as the settlement chain. The messaging collector pages events in chunks of
    // 200, so bump the RPC event page size as the settlement-chain messaging test does.
    let mut config = katana_utils::node::test_config();
    config.rpc.max_event_page_size = Some(1024);
    let node = TestNode::new_with_config(config).await;

    let appchain_chain_id = cairo_short_string_to_felt("KATANA_E2E").unwrap();

    // Deploy the real piltover settlement contract via the init flow (StarknetOs mode).
    let outcome = deploy_settlement_contract(
        settlement_account(&node),
        appchain_chain_id,
        FACT_REGISTRY,
        false,
    )
    .await
    .expect("deploy settlement contract");
    let contract = outcome.contract_address;

    // The Starknet → Appchain nonce climbs monotonically from 0 as messages are sent.
    let send_account = node.account();
    let nonce0 = send_message_nonce(&node, &send_account, contract).await;
    let nonce1 = send_message_nonce(&node, &send_account, contract).await;
    assert_eq!(nonce0, Felt::ZERO, "first message should use nonce 0");
    assert_eq!(nonce1, Felt::ONE, "second message should use nonce 1");

    // Reset the contract to genesis (the worker behind `katana init rollup --force`).
    reset_settlement_contract(settlement_account(&node), contract)
        .await
        .expect("reset settlement contract");

    // The state checkpoint is back to its constructor/genesis values: (state_root, block_number,
    // block_hash) == (0, Felt::MAX, 0). Felt::MAX is the genesis sentinel block number.
    let reader = AppchainContractReader::new(contract.into(), node.starknet_provider());
    let (state_root, block_number, block_hash) =
        reader.get_state().call().await.expect("get_state");
    assert_eq!(state_root, Felt::ZERO, "state root should reset to genesis");
    assert_eq!(block_number, Felt::MAX, "block number should reset to the genesis sentinel");
    assert_eq!(block_hash, Felt::ZERO, "block hash should reset to genesis");

    // And the message nonce restarts at 0 — as if the contract were freshly deployed.
    let nonce_after_reset = send_message_nonce(&node, &send_account, contract).await;
    assert_eq!(
        nonce_after_reset,
        Felt::ZERO,
        "message nonce should restart at 0 after reset_to_genesis",
    );
}
