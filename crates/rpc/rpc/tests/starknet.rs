use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use assert_matches::assert_matches;
use cainome::rs::{abigen, abigen_legacy};
use common::split_felt;
use indexmap::IndexSet;
use katana_primitives::event::ContinuationToken;
use katana_primitives::genesis::constant::{
    DEFAULT_ACCOUNT_CLASS_HASH, DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_PREFUNDED_ACCOUNT_BALANCE,
    DEFAULT_STRK_FEE_TOKEN_ADDRESS, DEFAULT_UDC_ADDRESS,
};
use katana_rpc_api::dev::DevApiClient;
use katana_utils::TestNode;
use num_traits::ToPrimitive;
use starknet::accounts::{
    Account, AccountError, AccountFactory, ConnectedAccount, ExecutionEncoding,
    OpenZeppelinAccountFactory as OZAccountFactory, SingleOwnerAccount,
};
use starknet::core::types::{
    BlockId, BlockTag, Call, DeclareTransactionReceipt, DeployAccountTransactionReceipt,
    EventFilter, EventsPage, ExecutionResult, Felt, MaybePendingBlockWithReceipts,
    MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, MaybePendingStateUpdate,
    StarknetError, TransactionExecutionStatus, TransactionFinalityStatus, TransactionReceipt,
    TransactionTrace,
};
use starknet::core::utils::get_contract_address;
use starknet::macros::{felt, selector};
use starknet::providers::{Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};
use tokio::sync::Mutex;

mod common;

#[tokio::test]
async fn declare_and_deploy_contract() -> Result<()> {
    let sequencer = TestNode::new().await;

    let account = sequencer.account();
    let provider = sequencer.starknet_provider();

    let path: PathBuf = PathBuf::from("tests/test_data/cairo1_contract.json");
    let (contract, compiled_class_hash) = common::prepare_contract_declaration_params(&path)?;

    let class_hash = contract.class_hash();
    let res = account.declare_v3(contract.into(), compiled_class_hash).send().await?;

    // check that the tx is executed successfully and return the correct receipt
    let receipt = katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
    assert_matches!(receipt.receipt, TransactionReceipt::Declare(DeclareTransactionReceipt { .. }));

    // check that the class is actually declared
    assert!(provider.get_class(BlockId::Tag(BlockTag::Pending), class_hash).await.is_ok());

    // check state update includes class in declared_classes
    let state_update = provider.get_state_update(BlockId::Tag(BlockTag::Latest)).await?;
    match state_update {
        MaybePendingStateUpdate::Update(update) => {
            assert!(update
                .state_diff
                .declared_classes
                .iter()
                .any(|item| item.class_hash == class_hash
                    && item.compiled_class_hash == compiled_class_hash));
        }
        _ => panic!("Expected Update, got PendingUpdate"),
    }

    let ctor_args = vec![Felt::ONE, Felt::TWO];
    let calldata = [
        vec![
            res.class_hash,              // class hash
            Felt::ZERO,                  // salt
            Felt::ZERO,                  // unique
            Felt::from(ctor_args.len()), // constructor calldata len
        ],
        ctor_args.clone(),
    ]
    .concat();

    // pre-compute the contract address of the would-be deployed contract
    let address = get_contract_address(Felt::ZERO, res.class_hash, &ctor_args, Felt::ZERO);

    let res = account
        .execute_v3(vec![Call {
            calldata,
            to: DEFAULT_UDC_ADDRESS.into(),
            selector: selector!("deployContract"),
        }])
        .send()
        .await?;

    // wait for the tx to be mined
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;

    // make sure the contract is deployed
    let res = provider.get_class_hash_at(BlockId::Tag(BlockTag::Pending), address).await?;
    assert_eq!(res, class_hash);

    Ok(())
}

#[tokio::test]
async fn declaring_already_existing_class() -> Result<()> {
    let sequencer = TestNode::new().await;

    let account = sequencer.account();
    let provider = sequencer.starknet_provider();

    let path = PathBuf::from("tests/test_data/cairo1_contract.json");
    let (contract, compiled_hash) = common::prepare_contract_declaration_params(&path)?;
    let class_hash = contract.class_hash();

    // Declare the class for the first time.
    let res = account.declare_v3(contract.clone().into(), compiled_hash).send().await?;

    // check that the tx is executed successfully and return the correct receipt
    let _ = katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
    // check that the class is actually declared
    assert!(provider.get_class(BlockId::Tag(BlockTag::Pending), class_hash).await.is_ok());

    // -----------------------------------------------------------------------
    // Declaring the same class again should fail with a ClassAlreadyDeclared error

    // We set max fee manually to avoid perfoming fee estimation as we just want to test that the
    // pool validation will reject the tx.
    //
    // The value of the max fee is also irrelevant here, as the validator will only perform static
    // checks and will not run the account's validation.

    let result = account
        .declare_v3(contract.into(), compiled_hash)
        .l1_gas(1)
        .l2_gas(1)
        .l1_data_gas(1)
        .send()
        .await;

    assert_account_starknet_err!(result.unwrap_err(), StarknetError::ClassAlreadyDeclared);

    Ok(())
}

#[rstest::rstest]
#[tokio::test]
async fn deploy_account(
    #[values(true, false)] disable_fee: bool,
    #[values(None, Some(1000))] block_time: Option<u64>,
) {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.dev.fee = !disable_fee;
    config.sequencing.block_time = block_time;

    let sequencer = TestNode::new_with_config(config).await;

    let provider = sequencer.starknet_provider();
    let funding_account = sequencer.account();
    let chain_id = provider.chain_id().await.unwrap();

    let signer = LocalWallet::from(SigningKey::from_random());
    let class = DEFAULT_ACCOUNT_CLASS_HASH;
    let salt = felt!("0x123");

    // starknet-rs's utility for deploying an OpenZeppelin account
    let factory = OZAccountFactory::new(class, chain_id, &signer, &provider).await.unwrap();
    let deploy_account_tx = factory.deploy_v3(salt);
    let account_address = deploy_account_tx.address();

    // Fund the new account
    abigen_legacy!(FeeToken, "crates/rpc/rpc/tests/test_data/erc20.json");
    let contract = FeeToken::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &funding_account);

    // send enough tokens to the new_account's address just to send the deploy account tx
    let amount = Uint256 { low: felt!("0x5ea0fb889c9400"), high: Felt::ZERO };
    let res = contract.transfer(&account_address, &amount).send().await.unwrap();
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();

    // send the deploy account transaction
    let res = deploy_account_tx.send().await.unwrap();
    // the contract address in the send tx result must be the same as the computed one
    assert_eq!(res.contract_address, account_address);

    let receipt = katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();
    assert_matches!(
        receipt.receipt,
        TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt { contract_address, .. })  => {
            // the contract address in the receipt must be the same as the computed one
            assert_eq!(contract_address, account_address)
        }
    );

    // Verify the `getClassHashAt` returns the same class hash that we use for the account
    // deployment
    let res =
        provider.get_class_hash_at(BlockId::Tag(BlockTag::Pending), account_address).await.unwrap();
    assert_eq!(res, class);

    // deploy from empty balance,
    // need to test this case because of how blockifier's StatefulValidator works.
    // TODO: add more descriptive reason
    if disable_fee {
        let salt = felt!("0x456");

        // starknet-rs's utility for deploying an OpenZeppelin account
        let deploy_account_tx = factory.deploy_v3(salt);
        let account_address = deploy_account_tx.address();

        // send the tx
        let res = deploy_account_tx.send().await.unwrap();
        // the contract address in the send tx result must be the same as the computed one
        assert_eq!(res.contract_address, account_address);

        let receipt = katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();
        assert_matches!(
            receipt.receipt,
            TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt { contract_address, .. })  => {
                // the contract address in the receipt must be the same as the computed one
                assert_eq!(contract_address, account_address)
            }
        );

        // Verify the `getClassHashAt` returns the same class hash that we use for the account
        // deployment
        let res = provider
            .get_class_hash_at(BlockId::Tag(BlockTag::Pending), account_address)
            .await
            .unwrap();
        assert_eq!(res, class);
    }
}

abigen_legacy!(Erc20Contract, "crates/rpc/rpc/tests/test_data/erc20.json", derives(Clone));

#[tokio::test]
async fn estimate_fee() {
    let sequencer = TestNode::new().await;

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    // estimate fee with current nonce (the expected nonce)
    let address = account.address();
    let nonce = provider.get_nonce(BlockId::Tag(BlockTag::Pending), address).await.unwrap();
    let result = contract.transfer(&recipient, &amount).nonce(nonce).estimate_fee().await;
    assert!(result.is_ok(), "estimate should succeed with nonce == current nonce");
}

#[tokio::test]
async fn estimate_fee_with_small_nonce() {
    let sequencer = TestNode::new().await;

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    // send a valid transaction first to increment the nonce (so that we can test nonce < current
    // nonce later)
    let res = contract.transfer(&recipient, &amount).send().await.unwrap();
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();

    // estimate fee with arbitrary nonce < current nonce
    //
    // here we're essentially estimating a transaction with a nonce that has already been
    // used, so it should fail.
    let address = account.address();
    let current_nonce = provider.get_nonce(BlockId::Tag(BlockTag::Pending), address).await.unwrap();

    let nonce = current_nonce - 1;
    let result = contract.transfer(&recipient, &amount).nonce(nonce).estimate_fee().await;
    assert!(result.is_err(), "estimate should fail with nonce < current nonce");
}

#[tokio::test]
async fn estimate_fee_with_big_nonce() {
    let sequencer = TestNode::new().await;

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    // send a valid transaction first to increment the nonce (so that we can test nonce > current
    // nonce later)
    let res = contract.transfer(&recipient, &amount).send().await.unwrap();
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();

    // estimate fee with arbitrary nonce >= current nonce
    let nonce = felt!("0x1337");
    let result = contract.transfer(&recipient, &amount).nonce(nonce).estimate_fee().await;
    assert!(result.is_ok(), "estimate should succeed with nonce >= current nonce");
}

#[tokio::test]
async fn estimate_fee_on_reverted_transaction() {
    let sequencer = TestNode::new().await;
    let account = sequencer.account();

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: Felt::MAX, high: Felt::MAX };

    let error = contract.transfer(&recipient, &amount).estimate_fee().await.unwrap_err();

    assert_matches!(
        error,
        AccountError::Provider(ProviderError::StarknetError(
            StarknetError::TransactionExecutionError(err)
        )) => {
            assert_eq!(err.transaction_index, 0, "there's only one transaction");
        }
    )
}

#[rstest::rstest]
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_transactions_submissions(
    #[values(None, Some(1000))] block_time: Option<u64>,
) -> Result<()> {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.sequencing.block_time = block_time;
    let sequencer = TestNode::new_with_config(config).await;

    let provider = sequencer.starknet_provider();
    let account = Arc::new(sequencer.account());

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    let initial_nonce =
        provider.get_nonce(BlockId::Tag(BlockTag::Pending), sequencer.account().address()).await?;

    const N: usize = 100;
    let nonce = Arc::new(Mutex::new(initial_nonce));
    let txs = Arc::new(Mutex::new(IndexSet::with_capacity(N)));

    let mut handles = Vec::with_capacity(N);

    for _ in 0..N {
        let txs = txs.clone();
        let nonce = nonce.clone();
        let amount = amount.clone();
        let account = account.clone();

        let handle = tokio::spawn(async move {
            let mut nonce = nonce.lock().await;
            let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), account);

            let res = contract.transfer(&recipient, &amount).nonce(*nonce).send().await.unwrap();

            txs.lock().await.insert(res.transaction_hash);
            *nonce += Felt::ONE;
        });

        handles.push(handle);
    }

    // wait for all txs to be submitted
    for handle in handles {
        handle.await?;
    }

    // Wait only for the last transaction to be accepted
    let txs = txs.lock().await;
    let last_tx = txs.last().unwrap();
    katana_utils::TxWaiter::new(*last_tx, &provider).await?;

    // we should've submitted ITERATION transactions
    assert_eq!(txs.len(), N);

    // check the status of each txs
    for hash in txs.iter() {
        let receipt = provider.get_transaction_receipt(hash).await?;
        assert_eq!(receipt.receipt.execution_result(), &ExecutionResult::Succeeded);
        assert_eq!(receipt.receipt.finality_status(), &TransactionFinalityStatus::AcceptedOnL2);
    }

    let nonce = account.get_nonce().await?;
    assert_eq!(nonce, Felt::from(N), "Nonce should be incremented by {N} time");

    Ok(())
}

#[rstest::rstest]
#[tokio::test]
async fn ensure_validator_have_valid_state(#[values(None, Some(1000))] block_time: Option<u64>) {
    let mut config = katana_utils::node::test_config();
    config.dev.fee = true;
    config.sequencing.block_time = block_time;

    let sequencer = TestNode::new_with_config(config).await;
    let account = sequencer.account();

    // setup test contract to interact with.
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // reduce account balance
    let recipient = felt!("0x1337");
    let (low, high) = split_felt(Felt::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE / 2));
    let amount = Uint256 { low, high };

    let res = contract.transfer(&recipient, &amount).send().await.unwrap();
    katana_utils::TxWaiter::new(res.transaction_hash, &sequencer.starknet_provider())
        .await
        .unwrap();

    // this should fail validation due to insufficient balance because we specify max fee > the
    // actual balance that we have now.
    let err = contract
        .transfer(&recipient, &amount)
        .gas_estimate_multiplier(1000000.0)
        .send()
        .await
        .unwrap_err();

    assert_account_starknet_err!(err, StarknetError::InsufficientAccountBalance);
}

#[rstest::rstest]
#[tokio::test]
async fn send_txs_with_insufficient_fee(
    #[values(true, false)] disable_fee: bool,
    #[values(None, Some(1000))] block_time: Option<u64>,
) -> Result<()> {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.dev.fee = !disable_fee;
    config.sequencing.block_time = block_time;
    let sequencer = TestNode::new_with_config(config).await;

    // setup test contract to interact with.
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), sequencer.account());

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // initial sender's account nonce. use to assert how the txs validity change the account nonce.
    let initial_nonce = sequencer.account().get_nonce().await?;

    // -----------------------------------------------------------------------
    //  transaction with low max fee (underpriced).

    let res =
        contract.transfer(&recipient, &amount).l2_gas(1).l1_gas(1).l1_data_gas(1).send().await;

    if disable_fee {
        // In no fee mode, the transaction resources (ie max fee) is totally ignored. So doesn't
        // matter what value is set, the transaction will always be executed successfully.
        assert_matches!(res, Ok(tx) => {
            let tx_hash = tx.transaction_hash;
            assert_matches!(katana_utils::TxWaiter::new(tx_hash, &sequencer.starknet_provider()).await, Ok(_));
            assert_matches!(katana_utils::TxWaiter::new(tx_hash, &sequencer.starknet_provider()).await, Ok(_));
        });

        let nonce = sequencer.account().get_nonce().await?;
        assert_eq!(initial_nonce + 1, nonce, "Nonce should change in fee-disabled mode");
    } else {
        assert_account_starknet_err!(
            res.unwrap_err(),
            StarknetError::InsufficientResourcesForValidate
        );
        let nonce = sequencer.account().get_nonce().await?;
        assert_eq!(initial_nonce, nonce, "Nonce shouldn't change in fee-enabled mode");
    }

    // -----------------------------------------------------------------------
    //  transaction with insufficient balance.

    // Set the gas estimate multiplier high enough to artficially bump the total resource cost so
    // that it exceeds what the account can actually cover.
    let res =
        contract.transfer(&recipient, &amount).gas_estimate_multiplier(1000000.0).send().await;

    if disable_fee {
        // in no fee mode, account balance is ignored. as long as the max fee (aka resources) is
        // enough to at least run the account validation, the tx should be accepted.
        // Wait for the transaction to be accepted
        katana_utils::TxWaiter::new(res?.transaction_hash, &sequencer.starknet_provider()).await?;

        // nonce should be incremented by 1 after a valid tx.
        let nonce = sequencer.account().get_nonce().await?;
        assert_eq!(initial_nonce + 2, nonce, "Nonce should change in fee-disabled mode");
    } else {
        assert_account_starknet_err!(res.unwrap_err(), StarknetError::InsufficientAccountBalance);

        // nonce shouldn't change for an invalid tx.
        let nonce = sequencer.account().get_nonce().await?;
        assert_eq!(initial_nonce, nonce, "Nonce shouldn't change in fee-enabled mode");
    }

    Ok(())
}

#[rstest::rstest]
#[tokio::test]
async fn send_txs_with_invalid_signature(
    #[values(true, false)] disable_validate: bool,
    #[values(None, Some(1000))] block_time: Option<u64>,
) -> Result<()> {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.dev.account_validation = !disable_validate;
    config.sequencing.block_time = block_time;
    let sequencer = TestNode::new_with_config(config).await;

    // starknet-rs doesn't provide a way to manually set the signatures so instead we create an
    // account with random signer to simulate invalid signatures.

    let account = SingleOwnerAccount::new(
        sequencer.starknet_provider(),
        LocalWallet::from(SigningKey::from_random()),
        sequencer.account().address(),
        sequencer.starknet_provider().chain_id().await?,
        ExecutionEncoding::New,
    );

    // setup test contract to interact with.
    let contract = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // get the base fee
    let simulated = contract.transfer(&recipient, &amount).simulate(true, false).await.unwrap();
    let fee = simulated.fee_estimation;

    // initial sender's account nonce. use to assert how the txs validity change the account nonce.
    let initial_nonce = account.get_nonce().await?;

    // -----------------------------------------------------------------------
    //  transaction with invalid signatures.

    // we set the max fee manually here to skip fee estimation. we want to test the pool validator.
    let res = contract
        .transfer(&recipient, &amount)
        .l1_gas(fee.l1_gas_consumed.to_u64().unwrap())
        .l2_gas(fee.l2_gas_consumed.to_u64().unwrap())
        .l1_data_gas(fee.l1_data_gas_consumed.to_u64().unwrap())
        .l1_gas_price(fee.l1_gas_price.to_u128().unwrap())
        .l2_gas_price(fee.l2_gas_price.to_u128().unwrap())
        .l1_data_gas_price(fee.l1_data_gas_price.to_u128().unwrap())
        .send()
        .await;

    if disable_validate {
        // Wait for the transaction to be accepted
        katana_utils::TxWaiter::new(res?.transaction_hash, &sequencer.starknet_provider()).await?;

        // nonce should be incremented by 1 after a valid tx.
        let nonce = sequencer.account().get_nonce().await?;
        assert_eq!(initial_nonce + 1, nonce);
    } else {
        assert_account_starknet_err!(res.unwrap_err(), StarknetError::ValidationFailure(_));

        // nonce shouldn't change for an invalid tx.
        let nonce = sequencer.account().get_nonce().await?;
        assert_eq!(initial_nonce, nonce);
    }

    Ok(())
}

#[rstest::rstest]
#[tokio::test]
async fn send_txs_with_invalid_nonces(
    #[values(None, Some(1000))] block_time: Option<u64>,
) -> Result<()> {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.sequencing.block_time = block_time;
    let sequencer = TestNode::new_with_config(config).await;

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // setup test contract to interact with.
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // send a valid transaction first to increment the nonce (so that we can test nonce < current
    // nonce later)
    let res = contract.transfer(&recipient, &amount).send().await?;
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;

    // initial sender's account nonce. use to assert how the txs validity change the account nonce.
    let initial_nonce = account.get_nonce().await?;
    assert_eq!(initial_nonce, Felt::ONE, "Initial nonce after sending 1st tx should be 1.");

    // get the base fee
    let fee = contract.transfer(&recipient, &amount).estimate_fee().await.unwrap();

    // -----------------------------------------------------------------------
    //  transaction with nonce < account nonce.

    let old_nonce = initial_nonce - Felt::ONE;
    let res = contract
        .transfer(&recipient, &amount)
        .nonce(old_nonce)
        .l1_gas(fee.l1_gas_consumed.to_u64().unwrap())
        .l2_gas(fee.l2_gas_consumed.to_u64().unwrap())
        .l1_data_gas(fee.l1_data_gas_consumed.to_u64().unwrap())
        .l1_gas_price(fee.l1_gas_price.to_u128().unwrap())
        .l2_gas_price(fee.l2_gas_price.to_u128().unwrap())
        .l1_data_gas_price(fee.l1_data_gas_price.to_u128().unwrap())
        .send()
        .await;

    assert_account_starknet_err!(res.unwrap_err(), StarknetError::InvalidTransactionNonce);

    let nonce = account.get_nonce().await?;
    assert_eq!(nonce, initial_nonce, "Nonce shouldn't change on invalid tx.");

    // -----------------------------------------------------------------------
    //  transaction with nonce = account nonce.

    let curr_nonce = initial_nonce;
    let res = contract
        .transfer(&recipient, &amount)
        .nonce(curr_nonce)
        .l1_gas(fee.l1_gas_consumed.to_u64().unwrap())
        .l2_gas(fee.l2_gas_consumed.to_u64().unwrap())
        .l1_data_gas(fee.l1_data_gas_consumed.to_u64().unwrap())
        .l1_gas_price(fee.l1_gas_price.to_u128().unwrap())
        .l2_gas_price(fee.l2_gas_price.to_u128().unwrap())
        .l1_data_gas_price(fee.l1_data_gas_price.to_u128().unwrap())
        .send()
        .await?;

    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;

    let nonce = account.get_nonce().await?;
    assert_eq!(nonce, Felt::TWO, "Nonce should be 2 after sending two valid txs.");

    // -----------------------------------------------------------------------
    //  transaction with nonce >= account nonce.
    //
    // ideally, tx with nonce >= account nonce should be considered as valid BUT not to be executed
    // immediately and should be kept around in the pool until the nonce is reached. however,
    // katana doesn't support this feature yet so the current behaviour is to treat the tx as
    // invalid with nonce mismatch error.

    let new_nonce = felt!("0x100");
    let res = contract
        .transfer(&recipient, &amount)
        .nonce(new_nonce)
        .l1_gas(fee.l1_gas_consumed.to_u64().unwrap())
        .l2_gas(fee.l2_gas_consumed.to_u64().unwrap())
        .l1_data_gas(fee.l1_data_gas_consumed.to_u64().unwrap())
        .l1_gas_price(fee.l1_gas_price.to_u128().unwrap())
        .l2_gas_price(fee.l2_gas_price.to_u128().unwrap())
        .l1_data_gas_price(fee.l1_data_gas_price.to_u128().unwrap())
        .send()
        .await;

    assert_account_starknet_err!(res.unwrap_err(), StarknetError::InvalidTransactionNonce);

    let nonce = account.get_nonce().await?;
    assert_eq!(nonce, Felt::TWO, "Nonce shouldn't change bcs the tx is still invalid.");

    Ok(())
}

// TODO: write more elaborate tests for get events.
#[tokio::test]
async fn get_events_no_pending() -> Result<()> {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = true;
    let sequencer = TestNode::new_with_config(config).await;

    // create a json rpc client to interact with the dev api.
    let client = sequencer.rpc_http_client();

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // setup test contract to interact with.
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);
    // tx that emits 1 event
    let tx = || contract.transfer(&Felt::ONE, &Uint256 { low: Felt::ONE, high: Felt::ZERO });

    const BLOCK_1_TX_COUNT: usize = 5;
    const EVENT_COUNT_PER_TX: usize = 1;
    const TOTAL_EVENT_COUNT: usize = BLOCK_1_TX_COUNT * EVENT_COUNT_PER_TX;

    for _ in 0..BLOCK_1_TX_COUNT {
        let res = tx().send().await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
    }

    // generate a block to mine pending transactions.
    client.generate_block().await?;

    let filter = EventFilter {
        keys: None,
        address: None,
        to_block: Some(BlockId::Number(1)),
        from_block: Some(BlockId::Number(0)),
    };

    // -----------------------------------------------------------------------
    //  case 1 (chunk size = 0)

    let chunk_size = 0;
    let EventsPage { events, continuation_token } =
        provider.get_events(filter.clone(), None, chunk_size).await?;

    assert_eq!(events.len(), 0);
    assert_matches!(continuation_token, Some(token ) => {
        let token = ContinuationToken::parse(&token)?;
        assert_eq!(token.block_n, 1);
        assert_eq!(token.txn_n, 0);
        assert_eq!(token.event_n, 0);
    });

    // -----------------------------------------------------------------------
    //  case 2

    let chunk_size = 3;
    let EventsPage { events, continuation_token } =
        provider.get_events(filter.clone(), None, chunk_size).await?;

    assert_eq!(events.len(), 3, "Total events should be limited by chunk size ({chunk_size})");
    assert_matches!(continuation_token, Some(ref token) => {
        let token = ContinuationToken::parse(token)?;
        assert_eq!(token.block_n, 1);
        assert_eq!(token.txn_n, 3);
        assert_eq!(token.event_n, 0);
    });

    let EventsPage { events, continuation_token } =
        provider.get_events(filter.clone(), continuation_token, chunk_size).await?;

    assert_eq!(events.len(), 2, "Remaining should be 2");
    assert_matches!(continuation_token, None);

    // -----------------------------------------------------------------------
    //  case 3 (max chunk is greater than total events in the requested range)

    let chunk_size = 100;
    let EventsPage { events, continuation_token } =
        provider.get_events(filter.clone(), None, chunk_size).await?;

    assert_eq!(events.len(), TOTAL_EVENT_COUNT);
    assert_matches!(continuation_token, None);

    Ok(())
}

#[tokio::test]
async fn get_events_with_pending() -> Result<()> {
    // setup test sequencer with the given configuration
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = true;
    let sequencer = TestNode::new_with_config(config).await;

    // create a json rpc client to interact with the dev api.
    let client = sequencer.rpc_http_client();

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // setup test contract to interact with.
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);
    // tx that emits 1 event
    let tx = || contract.transfer(&Felt::ONE, &Uint256 { low: Felt::ONE, high: Felt::ZERO });

    const BLOCK_1_TX_COUNT: usize = 5;
    const PENDING_BLOCK_TX_COUNT: usize = 5;

    for _ in 0..BLOCK_1_TX_COUNT {
        let res = tx().send().await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
    }

    // generate block 1
    client.generate_block().await?;

    // events in pending block (2)
    for _ in 0..PENDING_BLOCK_TX_COUNT {
        let res = tx().send().await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
    }

    // because we didnt specifically set the `from` and `to` block, it will implicitly
    // get events starting from the initial (0) block to the pending block (2)
    let filter = EventFilter { keys: None, address: None, to_block: None, from_block: None };

    let chunk_size = BLOCK_1_TX_COUNT;
    let EventsPage { events, continuation_token } =
        provider.get_events(filter.clone(), None, chunk_size as u64).await?;

    assert_eq!(events.len(), chunk_size);
    assert_matches!(continuation_token, Some(ref token) => {
        // the continuation token should now point to block 2 (pending block) because:-
        // (1) the filter doesn't specify the exact 'to' block, so it will keep moving the cursor to point to the next block.
        // (2) events in block 1 has been exhausted by the first two queries.
        let token = ContinuationToken::parse(token)?;
        assert_eq!(token.block_n, 2);
        assert_eq!(token.txn_n, 0);
        assert_eq!(token.event_n, 0);
    });

    // we split the pending events into two chunks to cover different cases.

    let chunk_size = 3;
    let EventsPage { events, continuation_token } =
        provider.get_events(filter.clone(), continuation_token, chunk_size).await?;

    assert_eq!(events.len() as u64, chunk_size);
    assert_matches!(continuation_token, Some(ref token) => {
        let token = ContinuationToken::parse(token)?;
        assert_eq!(token.block_n, 2);
        assert_eq!(token.txn_n, 3);
        assert_eq!(token.event_n, 0);
    });

    // get the rest of events in the pending block
    let EventsPage { events, continuation_token } =
        provider.get_events(filter.clone(), continuation_token, chunk_size).await?;

    assert_eq!(events.len(), PENDING_BLOCK_TX_COUNT - chunk_size as usize);
    assert_matches!(continuation_token, Some(ref token) => {
        let token = ContinuationToken::parse(token)?;
        assert_eq!(token.block_n, 2);
        assert_eq!(token.txn_n, 5);
        assert_eq!(token.event_n, 0);
    });

    // fetching events with the continuation token should return an empty list and the
    // token shouldn't change.
    let EventsPage { events, continuation_token: new_token } =
        provider.get_events(filter, continuation_token.clone(), chunk_size).await?;

    assert_eq!(events.len(), 0);
    assert_eq!(new_token, continuation_token);

    Ok(())
}

#[tokio::test]
async fn trace() -> Result<()> {
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = true;
    let sequencer = TestNode::new_with_config(config).await;

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();
    let rpc_client = sequencer.rpc_http_client();

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    // -----------------------------------------------------------------------
    // Transactions not in pending block

    let mut hashes = Vec::new();

    for _ in 0..2 {
        let res = contract.transfer(&recipient, &amount).send().await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
        hashes.push(res.transaction_hash);
    }

    // Generate a block to include the transactions. The generated block will have block number 1.
    rpc_client.generate_block().await?;

    for hash in hashes {
        let trace = provider.trace_transaction(hash).await?;
        assert_matches!(trace, TransactionTrace::Invoke(_));
    }

    // -----------------------------------------------------------------------
    // Transactions in pending block

    for _ in 0..2 {
        let res = contract.transfer(&recipient, &amount).send().await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;

        let trace = provider.trace_transaction(res.transaction_hash).await?;
        assert_matches!(trace, TransactionTrace::Invoke(_));
    }

    Ok(())
}

#[tokio::test]
async fn block_traces() -> Result<()> {
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = true;
    let sequencer = TestNode::new_with_config(config).await;

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();
    let rpc_client = sequencer.rpc_http_client();

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    let mut hashes = Vec::new();

    // -----------------------------------------------------------------------
    // Block 1

    for _ in 0..5 {
        let res = contract.transfer(&recipient, &amount).send().await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
        hashes.push(res.transaction_hash);
    }

    // Generate a block to include the transactions. The generated block will have block number 1.
    rpc_client.generate_block().await?;

    // Get the traces of the transactions in block 1.
    let block_id = BlockId::Number(1);
    let traces = provider.trace_block_transactions(block_id).await?;
    assert_eq!(traces.len(), 5);

    for i in 0..5 {
        assert_eq!(traces[i].transaction_hash, hashes[i]);
        assert_matches!(&traces[i].trace_root, TransactionTrace::Invoke(_));
    }

    // -----------------------------------------------------------------------
    // Block 2

    // remove the previous transaction hashes
    hashes.clear();

    for _ in 0..2 {
        let res = contract.transfer(&recipient, &amount).send().await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
        hashes.push(res.transaction_hash);
    }

    // Generate a block to include the transactions. The generated block will have block number 2.
    rpc_client.generate_block().await?;

    // Get the traces of the transactions in block 2.
    let block_id = BlockId::Number(2);
    let traces = provider.trace_block_transactions(block_id).await?;
    assert_eq!(traces.len(), 2);

    for i in 0..2 {
        assert_eq!(traces[i].transaction_hash, hashes[i]);
        assert_matches!(&traces[i].trace_root, TransactionTrace::Invoke(_));
    }

    // -----------------------------------------------------------------------
    // Block 3 (Pending)

    // remove the previous transaction hashes
    hashes.clear();

    for _ in 0..3 {
        let res = contract.transfer(&recipient, &amount).send().await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await?;
        hashes.push(res.transaction_hash);
    }

    // Get the traces of the transactions in block 3 (pending).
    let block_id = BlockId::Tag(BlockTag::Pending);
    let traces = provider.trace_block_transactions(block_id).await?;
    assert_eq!(traces.len(), 3);

    for i in 0..3 {
        assert_eq!(traces[i].transaction_hash, hashes[i]);
        assert_matches!(&traces[i].trace_root, TransactionTrace::Invoke(_));
    }

    Ok(())
}

// Test that the v3 transactions are working as expected. The expectation is that the v3 transaction
// will be using STRK fee token as its gas fee. So, the STRK fee token must exist in the chain in
// order for this to pass.
#[tokio::test]
async fn v3_transactions() {
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = true;
    let sequencer = TestNode::new_with_config(config).await;

    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // craft a raw v3 transaction, should probably use abigen for simplicity
    let to = DEFAULT_STRK_FEE_TOKEN_ADDRESS.into();
    let selector = selector!("transfer");
    let calldata = vec![felt!("0x1"), felt!("0x1"), Felt::ZERO];

    let res = account
        .execute_v3(vec![Call { to, selector, calldata }])
        .l2_gas(100000000000)
        .send()
        .await
        .unwrap();

    let rec = katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();
    let status = rec.receipt.execution_result().status();
    assert_eq!(status, TransactionExecutionStatus::Succeeded);
}

#[tokio::test]
async fn fetch_pending_blocks() {
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = true;
    let sequencer = TestNode::new_with_config(config).await;

    // create a json rpc client to interact with the dev api.
    let dev_client = sequencer.rpc_http_client();
    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    // list of tx hashes that we've sent
    let mut txs = Vec::new();

    for _ in 0..3 {
        let res = contract.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();
        katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();
        txs.push(res.transaction_hash);
    }

    let block_id = BlockId::Tag(BlockTag::Pending);

    // -----------------------------------------------------------------------

    let latest_block_hash_n_num = provider.block_hash_and_number().await.unwrap();
    let latest_block_hash = latest_block_hash_n_num.block_hash;

    let block_with_txs = provider.get_block_with_txs(block_id).await.unwrap();

    if let MaybePendingBlockWithTxs::PendingBlock(block) = block_with_txs {
        assert_eq!(block.transactions.len(), txs.len());
        assert_eq!(block.parent_hash, latest_block_hash);
        assert_eq!(txs[0], *block.transactions[0].transaction_hash());
        assert_eq!(txs[1], *block.transactions[1].transaction_hash());
        assert_eq!(txs[2], *block.transactions[2].transaction_hash());
    } else {
        panic!("expected pending block with transactions")
    }

    let block_with_tx_hashes = provider.get_block_with_tx_hashes(block_id).await.unwrap();
    if let MaybePendingBlockWithTxHashes::PendingBlock(block) = block_with_tx_hashes {
        assert_eq!(block.transactions.len(), txs.len());
        assert_eq!(block.parent_hash, latest_block_hash);
        assert_eq!(txs[0], block.transactions[0]);
        assert_eq!(txs[1], block.transactions[1]);
        assert_eq!(txs[2], block.transactions[2]);
    } else {
        panic!("expected pending block with transaction hashes")
    }

    let block_with_receipts = provider.get_block_with_receipts(block_id).await.unwrap();
    if let MaybePendingBlockWithReceipts::PendingBlock(block) = block_with_receipts {
        assert_eq!(block.transactions.len(), txs.len());
        assert_eq!(block.parent_hash, latest_block_hash);
        assert_eq!(txs[0], *block.transactions[0].receipt.transaction_hash());
        assert_eq!(txs[1], *block.transactions[1].receipt.transaction_hash());
        assert_eq!(txs[2], *block.transactions[2].receipt.transaction_hash());
    } else {
        panic!("expected pending block with transaction receipts")
    }

    // Close the current pending block
    dev_client.generate_block().await.unwrap();

    // -----------------------------------------------------------------------

    let latest_block_hash_n_num = provider.block_hash_and_number().await.unwrap();
    let latest_block_hash = latest_block_hash_n_num.block_hash;
    let block_with_txs = provider.get_block_with_txs(block_id).await.unwrap();

    assert_matches!(block_with_txs, MaybePendingBlockWithTxs::PendingBlock(_));
    if let MaybePendingBlockWithTxs::PendingBlock(block) = block_with_txs {
        assert_eq!(block.transactions.len(), 0);
        assert_eq!(block.parent_hash, latest_block_hash);
    } else {
        panic!("expected pending block with transactions")
    }

    let block_with_tx_hashes = provider.get_block_with_tx_hashes(block_id).await.unwrap();
    if let MaybePendingBlockWithTxHashes::PendingBlock(block) = block_with_tx_hashes {
        assert_eq!(block.transactions.len(), 0);
        assert_eq!(block.parent_hash, latest_block_hash);
    } else {
        panic!("expected pending block with transaction hashes")
    }

    let block_with_receipts = provider.get_block_with_receipts(block_id).await.unwrap();
    if let MaybePendingBlockWithReceipts::PendingBlock(block) = block_with_receipts {
        assert_eq!(block.transactions.len(), 0);
        assert_eq!(block.parent_hash, latest_block_hash);
    } else {
        panic!("expected pending block with transaction receipts")
    }
}

// Querying for pending blocks in instant mining mode will always return the last accepted block.
#[tokio::test]
async fn fetch_pending_blocks_in_instant_mode() {
    let sequencer = TestNode::new().await;

    // create a json rpc client to interact with the dev api.
    let dev_client = sequencer.rpc_http_client();
    let provider = sequencer.starknet_provider();
    let account = sequencer.account();

    // Get the latest block hash before sending the tx beacuse the tx will generate a new block.
    let latest_block_hash_n_num = provider.block_hash_and_number().await.unwrap();
    let latest_block_hash = latest_block_hash_n_num.block_hash;

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    let res = contract.transfer(&recipient, &amount).send().await.unwrap();
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();
    katana_utils::TxWaiter::new(res.transaction_hash, &provider).await.unwrap();

    let block_id = BlockId::Tag(BlockTag::Pending);

    // -----------------------------------------------------------------------

    let block_with_txs = provider.get_block_with_txs(block_id).await.unwrap();

    if let MaybePendingBlockWithTxs::Block(block) = block_with_txs {
        assert_eq!(block.transactions.len(), 1);
        assert_eq!(block.parent_hash, latest_block_hash);
        assert_eq!(*block.transactions[0].transaction_hash(), res.transaction_hash);
    } else {
        panic!("expected pending block with transactions")
    }

    let block_with_tx_hashes = provider.get_block_with_tx_hashes(block_id).await.unwrap();
    if let MaybePendingBlockWithTxHashes::Block(block) = block_with_tx_hashes {
        assert_eq!(block.transactions.len(), 1);
        assert_eq!(block.parent_hash, latest_block_hash);
        assert_eq!(block.transactions[0], res.transaction_hash);
    } else {
        panic!("expected pending block with transaction hashes")
    }

    let block_with_receipts = provider.get_block_with_receipts(block_id).await.unwrap();
    if let MaybePendingBlockWithReceipts::Block(block) = block_with_receipts {
        assert_eq!(block.transactions.len(), 1);
        assert_eq!(block.parent_hash, latest_block_hash);
        assert_eq!(*block.transactions[0].receipt.transaction_hash(), res.transaction_hash);
    } else {
        panic!("expected pending block with transaction receipts")
    }

    // Get the recently generated block from the sent tx
    let latest_block_hash_n_num = provider.block_hash_and_number().await.unwrap();
    let latest_block_hash = latest_block_hash_n_num.block_hash;

    // Generate an empty block
    dev_client.generate_block().await.unwrap();

    // -----------------------------------------------------------------------

    let block_with_txs = provider.get_block_with_txs(block_id).await.unwrap();

    if let MaybePendingBlockWithTxs::Block(block) = block_with_txs {
        assert_eq!(block.transactions.len(), 0);
        assert_eq!(block.parent_hash, latest_block_hash);
    } else {
        panic!("expected block with transactions")
    }

    let block_with_tx_hashes = provider.get_block_with_tx_hashes(block_id).await.unwrap();
    if let MaybePendingBlockWithTxHashes::Block(block) = block_with_tx_hashes {
        assert_eq!(block.transactions.len(), 0);
        assert_eq!(block.parent_hash, latest_block_hash);
    } else {
        panic!("expected block with transaction hashes")
    }

    let block_with_receipts = provider.get_block_with_receipts(block_id).await.unwrap();
    if let MaybePendingBlockWithReceipts::Block(block) = block_with_receipts {
        assert_eq!(block.transactions.len(), 0);
        assert_eq!(block.parent_hash, latest_block_hash);
    } else {
        panic!("expected block with transaction receipts")
    }
}

#[tokio::test]
async fn call_contract() {
    let sequencer = TestNode::new().await;

    let provider = sequencer.starknet_provider();
    let account = sequencer.account().address();

    // -----------------------------------------------------------------------
    // Call legacy contract

    let contract = Erc20ContractReader::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &provider);
    let _ = contract.name().call().await.unwrap();
    let _ = contract.balanceOf(&account).call().await.unwrap();

    // -----------------------------------------------------------------------
    // Call contract

    abigen!(
        AccountContract,
        [
          {
            "type": "function",
            "name": "get_public_key",
            "inputs": [],
            "outputs": [
              {
                "type": "core::felt252"
              }
            ],
            "state_mutability": "view"
          }
        ]
    );

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = AccountContractReader::new(account, &provider);
    let _ = contract.get_public_key().call().await.unwrap();
}

#[tokio::test]
#[rstest::rstest]
#[case::lower_than_current_nonce(Felt::ZERO, false)]
#[case::is_current_nonce(Felt::ONE, true)]
#[case::higher_than_current_nonce_1(felt!("0x1337"), true)]
#[case::higher_than_current_nonce_2(Felt::MAX, true)]
async fn simulate_should_skip_strict_nonce_check(#[case] nonce: Felt, #[case] should_ok: bool) {
    let sequencer = TestNode::new().await;

    let account = sequencer.account();
    let provider = sequencer.starknet_provider();

    // setup contract to interact with (can be any existing contract that can be interacted with)
    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);
    // setup contract function params
    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    // send a valid transaction first to increment the nonce (so that we can test nonce < current
    // nonce later)
    let res = contract.transfer(&recipient, &amount).send().await.expect("failed to send tx");
    katana_utils::TxWaiter::new(res.transaction_hash, &provider)
        .await
        .expect("failed to execute tx");

    // send a valid transaction first to increment the nonce (so that we can test nonce < current
    // nonce later)
    let res = contract.transfer(&recipient, &amount).nonce(nonce).simulate(false, false).await;
    assert_eq!(res.is_ok(), should_ok)
}
