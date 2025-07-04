use katana_utils_macro::mock_provider;
use starknet::core::types::{BlockId, Felt, MaybePendingBlockWithTxs, PendingBlockWithTxs};
use starknet::providers::Provider;

mock_provider! {
    TestMockProvider,

    fn get_block_with_txs: (_block_id) => {
        Ok(MaybePendingBlockWithTxs::PendingBlock(PendingBlockWithTxs {
            transactions: vec![],
            timestamp: 0,
            l1_gas_price: starknet::core::types::ResourcePrice { price_in_fri: Felt::ZERO, price_in_wei: Felt::ZERO },
            l1_data_gas_price: starknet::core::types::ResourcePrice { price_in_fri: Felt::ZERO, price_in_wei: Felt::ZERO },
            l2_gas_price: starknet::core::types::ResourcePrice { price_in_fri: Felt::ZERO, price_in_wei: Felt::ZERO },
            parent_hash: Felt::ZERO,
            sequencer_address: Felt::ZERO,
            starknet_version: "0.13.0".to_string(),
            l1_da_mode: starknet::core::types::L1DataAvailabilityMode::Calldata,
        }))
    },

    fn get_storage_at: (_, _, _) => {
        Ok(Felt::from(42u32))
    },

    fn chain_id: () => {
        Ok(Felt::from(1u32))
    }
}

#[tokio::test]
async fn test_mock_provider_implemented_methods() {
    let provider = TestMockProvider::new();

    // Test implemented methods
    let block_result =
        provider.get_block_with_txs(BlockId::Tag(starknet::core::types::BlockTag::Latest)).await;
    assert!(block_result.is_ok());

    let storage_result = provider
        .get_storage_at(
            Felt::from(1u32),
            Felt::from(2u32),
            BlockId::Tag(starknet::core::types::BlockTag::Latest),
        )
        .await;
    assert!(storage_result.is_ok());
    assert_eq!(storage_result.unwrap(), Felt::from(42u32));

    let chain_id_result = provider.chain_id().await;
    assert!(chain_id_result.is_ok());
    assert_eq!(chain_id_result.unwrap(), Felt::from(1u32));
}

#[tokio::test]
async fn test_mock_provider_unimplemented_methods() {
    let provider = TestMockProvider::new();

    // Test unimplemented method - should panic with unimplemented!()
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(async { provider.spec_version().await })
    }));

    assert!(result.is_err());
}

#[tokio::test]
async fn test_mock_provider_default_construction() {
    let provider = TestMockProvider::default();
    let chain_id_result = provider.chain_id().await;
    assert!(chain_id_result.is_ok());
    assert_eq!(chain_id_result.unwrap(), Felt::from(1u32));
}

// Test that we can create multiple different mock providers
mock_provider! {
    AnotherMockProvider,

    fn block_number: () => {
        Ok(12345u64)
    }
}

#[tokio::test]
async fn test_multiple_mock_providers() {
    let provider1 = TestMockProvider::new();
    let provider2 = AnotherMockProvider::new();

    // Each provider should have its own implementation
    let chain_id = provider1.chain_id().await.unwrap();
    assert_eq!(chain_id, Felt::from(1u32));

    let block_num = provider2.block_number().await.unwrap();
    assert_eq!(block_num, 12345u64);
}

// Test that verifies the fn keyword syntax works correctly
mock_provider! {
    FnKeywordTestProvider,

    fn spec_version: () => {
        Ok("0.13.0".to_string())
    },

    fn get_nonce: (_, _) => {
        Ok(Felt::from(1u32))
    }
}

#[tokio::test]
async fn test_fn_keyword_syntax() {
    let provider = FnKeywordTestProvider::new();

    let spec_version = provider.spec_version().await.unwrap();
    assert_eq!(spec_version, "0.13.0");

    let nonce = provider
        .get_nonce(BlockId::Tag(starknet::core::types::BlockTag::Latest), Felt::from(0x123u32))
        .await
        .unwrap();
    assert_eq!(nonce, Felt::from(1u32));
}
