use std::path::PathBuf;

use anyhow::Result;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{eth_address, felt, Felt};
use katana_rpc_types::MsgFromL1;
use katana_utils::{TestNode, TxWaiter};
use starknet::accounts::Account;
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::core::utils::get_contract_address;
use starknet::macros::selector;

mod common;

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
