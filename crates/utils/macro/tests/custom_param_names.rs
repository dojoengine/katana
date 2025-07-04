use katana_utils_macro::mock_provider;
use starknet::core::types::{BlockId, BlockTag, Felt};
use starknet::providers::Provider;

// Test that users can use custom parameter names instead of the exact Provider trait names
mock_provider! {
    CustomParamMock,

    fn get_storage_at: (addr, storage_key, _block) => {
        // Use the custom parameter names in the implementation
        println!("Custom params - addr: {}, key: {}", addr.as_ref(), storage_key.as_ref());
        Ok(addr.as_ref() + storage_key.as_ref())
    },

    fn get_class_hash_at: (_block_ref, contract_addr) => {
        // Different custom names for these parameters
        Ok(*contract_addr.as_ref())
    },

    fn chain_id: () => {
        Ok(Felt::from(1337u32))
    },

    fn get_nonce: (_, account_address) => {
        // Custom names that are more descriptive
        Ok(*account_address.as_ref())
    }
}

// Test with very short parameter names
mock_provider! {
    ShortParamMock,

    fn get_storage_at: (_, _, _) => {
        Ok(Felt::from(42u32))
    },

    fn get_transaction_by_block_id_and_index: (_, idx) => {
        // Use the custom short names
        Ok(starknet::core::types::Transaction::Invoke(
            starknet::core::types::InvokeTransaction::V1(
                starknet::core::types::InvokeTransactionV1 {
                    transaction_hash: Felt::from(idx),
                    max_fee: Felt::from(1000u32),
                    signature: vec![],
                    nonce: Felt::ZERO,
                    sender_address: Felt::from(123u32),
                    calldata: vec![],
                }
            )
        ))
    }
}

// Test with descriptive parameter names
mock_provider! {
    DescriptiveParamMock,

    fn estimate_fee: (_transaction_request, _simulation_options, _) => {
        Ok(vec![starknet::core::types::FeeEstimate {
            l1_gas_consumed: 21000u64,
            l1_gas_price: 1000000000u128,
            l1_data_gas_consumed: 128u64,
            l1_data_gas_price: 1u128,
            l2_gas_consumed: 5000u64,
            l2_gas_price: 500000000u128,
            overall_fee: 21000000000000u128,
            unit: starknet::core::types::PriceUnit::Wei,
        }])
    }
}

#[tokio::test]
async fn test_custom_parameter_names() {
    let provider = CustomParamMock::new();

    // Test that custom parameter names work correctly
    let storage_value = provider
        .get_storage_at(Felt::from(10u32), Felt::from(5u32), BlockId::Tag(BlockTag::Latest))
        .await
        .unwrap();
    assert_eq!(storage_value, Felt::from(15u32)); // 10 + 5

    let class_hash = provider
        .get_class_hash_at(BlockId::Tag(BlockTag::Latest), Felt::from(999u32))
        .await
        .unwrap();
    assert_eq!(class_hash, Felt::from(999u32));

    let chain_id = provider.chain_id().await.unwrap();
    assert_eq!(chain_id, Felt::from(1337u32));

    let nonce =
        provider.get_nonce(BlockId::Tag(BlockTag::Latest), Felt::from(777u32)).await.unwrap();
    assert_eq!(nonce, Felt::from(777u32));
}

#[tokio::test]
async fn test_short_parameter_names() {
    let provider = ShortParamMock::new();

    let storage_value = provider
        .get_storage_at(Felt::from(1u32), Felt::from(2u32), BlockId::Tag(BlockTag::Latest))
        .await
        .unwrap();
    assert_eq!(storage_value, Felt::from(42u32));

    let transaction = provider
        .get_transaction_by_block_id_and_index(BlockId::Tag(BlockTag::Latest), 123u64)
        .await
        .unwrap();

    // Verify the transaction was created with the custom parameter value
    if let starknet::core::types::Transaction::Invoke(
        starknet::core::types::InvokeTransaction::V1(tx),
    ) = transaction
    {
        assert_eq!(tx.transaction_hash, Felt::from(123u64));
    } else {
        panic!("Expected InvokeTransactionV1");
    }
}

#[tokio::test]
async fn test_descriptive_parameter_names() {
    let provider = DescriptiveParamMock::new();

    let fee_estimates = provider
        .estimate_fee(
            vec![], // empty transaction request
            vec![], // empty simulation options
            BlockId::Tag(BlockTag::Latest),
        )
        .await
        .unwrap();

    assert_eq!(fee_estimates.len(), 1);
    assert_eq!(fee_estimates[0].l1_gas_consumed, 21000u64);
}

#[test]
fn test_custom_param_provider_traits() {
    // Test that generated structs still implement expected traits
    let provider = CustomParamMock::new();
    assert!(format!("{:?}", provider).contains("CustomParamMock"));

    let provider2 = ShortParamMock::default();
    assert!(format!("{:?}", provider2).contains("ShortParamMock"));

    let provider3 = DescriptiveParamMock::new();
    let cloned = provider3.clone();
    assert!(format!("{:?}", cloned).contains("DescriptiveParamMock"));
}
