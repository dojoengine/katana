use std::path::PathBuf;
use std::time::Duration;

use alloy_primitives::{Uint, U256};
use alloy_provider::ProviderBuilder;
use alloy_sol_types::sol;
use anyhow::Result;
use cainome::rs::abigen;
use katana_messaging::MessagingConfig;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::utils::transaction::{
    compute_l1_handler_tx_hash, compute_l1_to_l2_message_hash,
};
use katana_primitives::{eth_address, felt, ContractAddress, Felt};
use katana_rpc_types::{Class, MsgFromL1};
use katana_utils::{TestNode, TxWaiter};
use rand::Rng;
use starknet::accounts::{Account, ConnectedAccount};
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::core::types::{Hash256, ReceiptBlock, Transaction, TransactionReceipt};
use starknet::core::utils::get_contract_address;
use starknet::macros::selector;
use starknet::providers::Provider;
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

abigen!(CairoMessagingContract, "crates/rpc/rpc-server/tests/test_data/cairo_l1_msg_contract.json");

#[tokio::test(flavor = "multi_thread")]
async fn test_messaging() {
    // TODO: If there's a way to get the endpoint of anvil from the `l1_provider`, we could
    // remove that and use default anvil to let the OS assign the port.
    let port: u16 = rand::thread_rng().gen_range(35000..65000);

    let l1_provider = ProviderBuilder::new()
        .connect_anvil_with_wallet_and_config(|anvil| anvil.port(port))
        .expect("failed to build eth provider");

    // Deploy the core messaging contract on L1
    let core_contract = StarknetContract::deploy(&l1_provider).await.unwrap();

    // Deploy test contract on L1 used to send/receive messages to/from L2
    let l1_test_contract = Contract1::deploy(&l1_provider, *core_contract.address()).await.unwrap();

    let messaging_config = MessagingConfig {
        settlement: katana_messaging::SettlementChainConfig::Ethereum {
            rpc_url: Url::parse(&format!("http://localhost:{}", port)).unwrap(),
            contract_address: core_contract.address().clone(),
        },
        interval: 2,
        from_block: 0,
        confirmation_depth: 0,
    };

    let mut config = katana_utils::node::test_config();
    config.messaging = Some(messaging_config);
    let sequencer = TestNode::new_with_config(config).await;

    let katana_account = sequencer.account();
    let rpc_client = sequencer.starknet_rpc_client();

    // Deploy test L2 contract that can send/receive messages to/from L1
    let l2_test_contract = {
        // Prepare contract declaration params
        let path = PathBuf::from("tests/test_data/cairo_l1_msg_contract.json");
        let (contract, compiled_hash) = common::prepare_contract_declaration_params(&path).unwrap();

        // Declare the contract
        let class_hash = contract.class_hash();
        let res = katana_account.declare_v3(contract.into(), compiled_hash).send().await.unwrap();

        // The waiter already checks that the transaction is accepted and succeeded on L2.
        TxWaiter::new(res.transaction_hash, &rpc_client).await.expect("declare tx failed");

        // Checks that the class was indeed declared
        let block_id = BlockIdOrTag::Latest;
        let actual_class = rpc_client.get_class(block_id, class_hash).await.unwrap();

        let Class::Sierra(class) = actual_class else { panic!("Invalid class type") };
        assert_eq!(class.hash(), class_hash, "invalid declared class"); // just to make sure the rpc returns the correct class

        // Compute the contract address
        let address = get_contract_address(Felt::ZERO, class_hash, &[], Felt::ZERO);

        // Deploy the contract using UDC
        let res = ContractFactory::new_with_udc(class_hash, &katana_account, UdcSelector::New)
            .deploy_v3(Vec::new(), Felt::ZERO, false)
            .send()
            .await
            .expect("Unable to deploy contract");

        // The waiter already checks that the transaction is accepted and succeeded on L2.
        TxWaiter::new(res.transaction_hash, &rpc_client).await.expect("deploy tx failed");

        // Checks that the class was indeed deployed with the correct class
        let actual_class_hash = rpc_client
            .get_class_hash_at(block_id, address.into())
            .await
            .expect("failed to get class hash at address");

        assert_eq!(actual_class_hash, class_hash, "invalid deployed class");

        address
    };

    // Send message from L1 to L2
    {
        // The L1 sender address
        let sender = l1_test_contract.address();
        // The L2 contract address to send the message to
        let recipient = ContractAddress::from(l2_test_contract);
        // The L2 contract function to call
        let selector = selector!("msg_handler_value");
        // The L2 contract function arguments
        let calldata = [123u8];
        // Get the current L1 -> L2 message nonce
        let nonce = core_contract.l1ToL2MessageNonce().call().await.expect("get nonce");

        // Send message to L2
        let call = l1_test_contract
            .sendMessage(
                recipient.into(),
                U256::from_be_bytes(selector.to_bytes_be()),
                calldata.iter().map(|x| U256::from(*x)).collect::<Vec<_>>(),
            )
            .gas(12000000)
            .value(Uint::from(1));

        let receipt = call
            .send()
            .await
            .expect("failed to send tx")
            .get_receipt()
            .await
            .expect("error getting transaction receipt");

        assert!(receipt.status(), "failed to send L1 -> L2 message");

        // Wait for the tx to be mined on L2 (Katana)
        tokio::time::sleep(Duration::from_secs(5)).await;

        // In an l1_handler transaction, the first element of the calldata is always the Ethereum
        // address of the sender (msg.sender).
        let mut l1_tx_calldata = vec![Felt::from_bytes_be_slice(sender.as_slice())];
        l1_tx_calldata.extend(calldata.iter().map(|x| Felt::from(*x)));

        // Compute transaction hash
        let tx_hash = compute_l1_handler_tx_hash(
            Felt::ZERO,
            recipient,
            selector,
            &l1_tx_calldata,
            sequencer.starknet_provider().chain_id().await.unwrap(),
            nonce.to::<u64>().into(),
        );

        // fetch the transaction
        let tx = katana_account
            .provider()
            .get_transaction_by_hash(tx_hash)
            .await
            .expect("failed to get l1 handler tx");

        let Transaction::L1Handler(ref tx) = tx else {
            panic!("invalid transaction type");
        };

        // Assert the transaction fields
        assert_eq!(tx.contract_address, recipient.into());
        assert_eq!(tx.entry_point_selector, selector);
        assert_eq!(tx.calldata, l1_tx_calldata);

        // fetch the receipt
        let receipt_res = katana_account
            .provider()
            .get_transaction_receipt(tx.transaction_hash)
            .await
            .expect("failed to get receipt");

        match receipt_res.block {
            ReceiptBlock::Block { .. } => {
                let TransactionReceipt::L1Handler(receipt) = receipt_res.receipt else {
                    panic!("invalid receipt type");
                };

                let msg_hash = compute_l1_to_l2_message_hash(
                    sender.as_slice().try_into().unwrap(),
                    recipient,
                    selector,
                    &calldata.iter().map(|x| Felt::from(*x)).collect::<Vec<_>>(),
                    Felt::from_bytes_be(&nonce.to_be_bytes()),
                );

                let msg_fee = core_contract
                    .l1ToL2Messages(msg_hash)
                    .call()
                    .await
                    .expect("failed to get msg fee");

                assert_ne!(msg_fee, U256::ZERO, "msg fee must be non-zero if exist");
                assert_eq!(receipt.message_hash, Hash256::from_bytes(msg_hash.0));
            }

            _ => {
                panic!("Error, No Receipt TransactionReceipt")
            }
        }
    }

    // Send message from L2 to L1 testing must be done using Saya or part of
    // it to ensure the settlement contract is test on piltover and its `update_state` method.
}

#[tokio::test]
async fn estimate_message_fee() -> Result<()> {
    let sequencer = TestNode::new().await;

    let account = sequencer.account();
    let rpc_client = sequencer.starknet_rpc_client();

    // Declare and deploy a l1 handler contract
    let path = PathBuf::from("tests/test_data/cairo_l1_msg_contract.json");
    let (contract, compiled_hash) = common::prepare_contract_declaration_params(&path)?;
    let class_hash = contract.class_hash();

    let res = account.declare_v3(contract.into(), compiled_hash).send().await?;
    TxWaiter::new(res.transaction_hash, &rpc_client).await?;

    // Deploy the contract using UDC
    let res = ContractFactory::new_with_udc(class_hash, &account, UdcSelector::New)
        .deploy_v3(Vec::new(), Felt::ZERO, false)
        .send()
        .await?;

    TxWaiter::new(res.transaction_hash, &rpc_client).await?;

    // Compute the contract address of the l1 handler contract
    let l1handler_address = get_contract_address(Felt::ZERO, class_hash, &[], Felt::ZERO);

    // This is the function signature of the #[l1handler] function we''re gonna call. Though the
    // function accepts two arguments, we're only gonna pass one argument, as the `from_address`
    // of the `MsgFromL1` will be automatically injected as part of the function calldata.
    //
    // See https://docs.starknet.io/documentation/architecture_and_concepts/Network_Architecture/messaging-mechanism/#l1-l2-messages.
    //
    // #[l1_handler]
    // fn msg_handler_value(ref self: ContractState, from_address: felt252, value: felt252)

    let entry_point_selector = selector!("msg_handler_value");
    let payload = vec![felt!("123")];
    let from_address = eth_address!("0x0000000000000000000000000000000000001337");
    let to_address = l1handler_address.into();

    let msg = MsgFromL1 { payload, to_address, entry_point_selector, from_address };

    let result = rpc_client.estimate_message_fee(msg, BlockIdOrTag::PreConfirmed).await;
    assert!(result.is_ok());

    // #[derive(Drop, Serde)]
    // struct MyData {
    //     a: felt252,
    //     b: felt252,
    // }
    //
    // #[l1_handler]
    // fn msg_handler_struct(ref self: ContractState, from_address: felt252, data: MyData)

    let entry_point_selector = selector!("msg_handler_struct");
    // [ MyData.a , MyData.b ]
    let payload = vec![felt!("1"), felt!("2")];
    let from_address = eth_address!("0x0000000000000000000000000000000000001337");
    let to_address = l1handler_address.into();

    let msg = MsgFromL1 { payload, to_address, entry_point_selector, from_address };

    let result = rpc_client.estimate_message_fee(msg, BlockIdOrTag::PreConfirmed).await;
    assert!(result.is_ok());

    Ok(())
}

// ==============================================================================
// `starknet_getMessagesStatus` business-logic tests.
//
// These exercise the RPC method without a real settlement chain. We populate
// the L1->L2 index directly via the provider trait and assert the RPC response.
// ==============================================================================

mod messages_status {
    use katana_primitives::B256;
    use katana_provider::api::messaging::MessagingL1ToL2IndexWriter;
    use katana_provider::{MutableProvider, ProviderFactory};
    use katana_rpc_api::starknet::StarknetApiClient;
    use katana_rpc_types::message::MessageFinalityStatus;
    use katana_rpc_types::ExecutionResult;
    use katana_utils::TestNode;
    use starknet::core::types::Felt;

    /// Record one or more `(l1_hash -> l2_hash)` mappings on the test node's
    /// underlying provider, committing each write so the RPC handler can read
    /// them back.
    fn record_mappings<P>(node: &TestNode<P>, mappings: &[([u8; 32], Felt)])
    where
        P: ProviderFactory + Clone,
        <P as ProviderFactory>::Provider: katana_provider::ProviderRO,
        <P as ProviderFactory>::ProviderMut:
            katana_provider::ProviderRW + MessagingL1ToL2IndexWriter,
    {
        let factory = node.handle().node().provider();
        for (l1_hash, l2_hash) in mappings {
            let tx = factory.provider_mut();
            tx.record_l1_to_l2(l1_hash, *l2_hash).expect("record l1->l2 mapping");
            MutableProvider::commit(tx).expect("commit mapping write");
        }
    }

    /// An L1 hash that has never been recorded yields an empty list, not an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_l1_hash_returns_empty_list() {
        let node = TestNode::new().await;
        let client = node.rpc_http_client();

        let unknown = B256::from([0xde; 32]);
        let statuses = client.get_messages_status(unknown).await.expect("rpc call succeeds");

        assert!(statuses.is_empty(), "expected no statuses for unknown L1 hash");
    }

    /// Recorded mappings whose L2 transactions don't exist (never reached the
    /// pool, never mined) are silently dropped from the response. The RPC
    /// returns whatever entries it could resolve, including an empty list when
    /// every entry is missing.
    #[tokio::test(flavor = "multi_thread")]
    async fn vanished_l2_txs_are_skipped() {
        let node = TestNode::new().await;
        let client = node.rpc_http_client();

        let l1_hash = [0x11u8; 32];
        let bogus_l2_a = Felt::from_hex("0xa1").unwrap();
        let bogus_l2_b = Felt::from_hex("0xa2").unwrap();
        record_mappings(&node, &[(l1_hash, bogus_l2_a), (l1_hash, bogus_l2_b)]);

        let statuses =
            client.get_messages_status(B256::from(l1_hash)).await.expect("rpc call succeeds");

        assert!(
            statuses.is_empty(),
            "expected vanished L2 txs to be filtered out, got {statuses:?}"
        );
    }

    /// When the recorded L2 transactions actually exist on chain, the RPC
    /// returns one status per mapping with the correct finality.
    ///
    /// We piggyback on the test node's existing genesis transactions: at startup
    /// every funded account has `AcceptedOnL2` deploy transactions visible in
    /// storage. We grab a couple of those tx hashes and treat them as if they
    /// were spawned by some synthetic L1 transaction.
    #[tokio::test(flavor = "multi_thread")]
    async fn returns_accepted_on_l2_for_mined_txs() {
        use std::path::PathBuf;

        let node = TestNode::new().await;
        let account = node.account();
        let rpc_client = node.starknet_rpc_client();
        let client = node.rpc_http_client();

        // Send two L2 transactions and wait for both to be mined. Instant mining
        // (the default test config) gets them to AcceptedOnL2 immediately.
        let path = PathBuf::from("tests/test_data/cairo1_contract.json");
        let (contract_a, compiled_a) =
            super::common::prepare_contract_declaration_params(&path).unwrap();
        let res_a =
            starknet::accounts::Account::declare_v3(&account, contract_a.into(), compiled_a)
                .send()
                .await
                .expect("declare A");
        katana_utils::TxWaiter::new(res_a.transaction_hash, &rpc_client).await.expect("wait A");

        let path = PathBuf::from("tests/test_data/cairo_l1_msg_contract.json");
        let (contract_b, compiled_b) =
            super::common::prepare_contract_declaration_params(&path).unwrap();
        let res_b =
            starknet::accounts::Account::declare_v3(&account, contract_b.into(), compiled_b)
                .send()
                .await
                .expect("declare B");
        katana_utils::TxWaiter::new(res_b.transaction_hash, &rpc_client).await.expect("wait B");

        let l2_a = res_a.transaction_hash;
        let l2_b = res_b.transaction_hash;

        // Pretend both L2 txs were spawned from the same synthetic L1 transaction.
        let l1_hash = [0x42u8; 32];
        record_mappings(&node, &[(l1_hash, l2_a), (l1_hash, l2_b)]);

        let statuses =
            client.get_messages_status(B256::from(l1_hash)).await.expect("rpc call succeeds");

        assert_eq!(statuses.len(), 2, "expected 2 statuses, got {}", statuses.len());

        for status in &statuses {
            assert!(
                matches!(status.finality_status, MessageFinalityStatus::AcceptedOnL2),
                "expected AcceptedOnL2, got {:?}",
                status.finality_status
            );
            assert!(
                matches!(status.execution_result, ExecutionResult::Succeeded),
                "expected Succeeded, got {:?}",
                status.execution_result
            );
        }

        // Both recorded L2 hashes appear in the response.
        let returned: std::collections::HashSet<Felt> =
            statuses.iter().map(|s| s.transaction_hash).collect();
        assert!(returned.contains(&l2_a), "missing l2_a in response");
        assert!(returned.contains(&l2_b), "missing l2_b in response");
    }
}
