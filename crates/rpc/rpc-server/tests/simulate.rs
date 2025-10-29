use cainome::rs::abigen_legacy;
use katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{felt, Felt};
use katana_utils::TestNode;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::providers::Provider;
use starknet::signers::{LocalWallet, SigningKey};

abigen_legacy!(Erc20Contract, "crates/contracts/build/legacy/erc20.json", derives(Clone));

#[tokio::test]
async fn simulate() {
    let sequencer = TestNode::new().await;
    let account = sequencer.account();

    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    let result = contract.transfer(&recipient, &amount).simulate(false, false).await;
    assert!(result.is_ok(), "simulate should succeed");
}

#[rstest::rstest]
#[tokio::test]
async fn simulate_nonce_validation(#[values(None, Some(1000))] block_time: Option<u64>) {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.sequencing.block_time = block_time;
    let sequencer = TestNode::new_with_config(config).await;
    let provider = sequencer.starknet_rpc_client();
    let account = sequencer.account();

    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    // send a valid transaction first to increment the nonce (so that we can test nonce < current
    // nonce later)
    let res = contract.transfer(&recipient, &amount).send().await.unwrap();
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();

    // simulate with current nonce (the expected nonce)
    let nonce =
        provider.get_nonce(BlockIdOrTag::PreConfirmed, account.address().into()).await.unwrap();
    let result = contract.transfer(&recipient, &amount).nonce(nonce).simulate(false, false).await;
    assert!(result.is_ok(), "estimate should succeed with nonce == current nonce");

    // simulate with arbitrary nonce < current nonce
    //
    // here we're essentially simulating a transaction with a nonce that has already been
    // used, so it should fail.
    let nonce = nonce - 1;
    let result = contract.transfer(&recipient, &amount).nonce(nonce).simulate(false, false).await;
    assert!(result.is_err(), "estimate should fail with nonce < current nonce");

    // simulate with arbitrary nonce >= current nonce
    let nonce = felt!("0x1337");
    let result = contract.transfer(&recipient, &amount).nonce(nonce).simulate(false, false).await;
    assert!(result.is_ok(), "estimate should succeed with nonce >= current nonce");
}

#[rstest::rstest]
#[tokio::test]
async fn simulate_with_insufficient_fee(
    #[values(true, false)] disable_node_fee: bool,
    #[values(None, Some(1000))] block_time: Option<u64>,
) {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.dev.fee = !disable_node_fee;
    config.sequencing.block_time = block_time;
    let sequencer = TestNode::new_with_config(config).await;

    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), sequencer.account());

    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // -----------------------------------------------------------------------
    //  transaction with low max fee (underpriced).

    let res = contract
        .transfer(&recipient, &amount)
        .l2_gas(1)
        .l2_gas_price(1)
        .l1_gas(1)
        .l1_gas_price(1)
        .l1_data_gas(1)
        .l1_data_gas_price(1)
        .simulate(false, false)
        .await;

    if disable_node_fee {
        assert!(res.is_ok(), "should succeed when fee is disabled");
    } else {
        assert!(res.is_err(), "should fail when fee is enabled");
    }

    // simulate with 'skip fee charge' flag
    let result = contract.transfer(&recipient, &amount).simulate(false, true).await;
    assert!(result.is_ok(), "should succeed no matter");

    // -----------------------------------------------------------------------
    //  transaction with insufficient balance.

    let result = contract
        .transfer(&recipient, &amount)
        .l2_gas(u64::MAX)
        .l2_gas_price(1)
        .l1_gas(u64::MAX)
        .l1_gas_price(1)
        .l1_data_gas(u64::MAX)
        .l1_data_gas_price(1)
        .simulate(false, false)
        .await;

    if disable_node_fee {
        assert!(result.is_ok(), "estimate should succeed when fee is disabled");
    } else {
        assert!(result.is_err(), "should fail when fee is enabled");
    }

    // simulate with 'skip fee charge' flag
    let result = contract
        .transfer(&recipient, &amount)
        .l2_gas(u64::MAX)
        .l2_gas_price(1)
        .l1_gas(u64::MAX)
        .l1_gas_price(1)
        .l1_data_gas(u64::MAX)
        .l1_data_gas_price(1)
        .simulate(false, true)
        .await;

    assert!(result.is_ok(), "should succeed no matter");
}

#[rstest::rstest]
#[tokio::test]
async fn simulate_with_invalid_signature(
    #[values(true, false)] disable_node_validate: bool,
    #[values(None, Some(1000))] block_time: Option<u64>,
) {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.dev.account_validation = !disable_node_validate;
    config.sequencing.block_time = block_time;
    let sequencer = TestNode::new_with_config(config).await;

    // starknet-rs doesn't provide a way to manually set the signatures so instead we create an
    // account with random signer to simulate invalid signatures.

    let account = SingleOwnerAccount::new(
        sequencer.starknet_provider(),
        LocalWallet::from(SigningKey::from_random()),
        sequencer.account().address(),
        sequencer.starknet_provider().chain_id().await.unwrap(),
        ExecutionEncoding::New,
    );

    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };
    let result = contract.transfer(&recipient, &amount).simulate(false, false).await;

    if disable_node_validate {
        assert!(result.is_ok(), "should succeed when validate is disabled");
    } else {
        assert!(result.is_err(), "should fail when validate is enabled");
    }

    // simulate with 'skip validate' flag
    let result = contract.transfer(&recipient, &amount).simulate(true, false).await;
    assert!(result.is_ok(), "should succeed no matter");
}
