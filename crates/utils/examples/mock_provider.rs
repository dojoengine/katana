//! Example demonstrating the mock_provider macro for creating mock Provider implementations.
//!
//! This example shows how to use the `mock_provider!` macro to create mock implementations
//! of the Starknet Provider trait for testing purposes.
//!
//! Run this example with:
//! ```bash
//! cargo run --example mock_provider_example
//! ```

use katana_utils::mock_provider;
use starknet::core::types::{
    BlockId, BlockTag, Felt, L1DataAvailabilityMode, MaybePreConfirmedBlockWithTxs,
    PreConfirmedBlockWithTxs, ResourcePrice,
};
use starknet::providers::Provider;

// Create a mock provider that only implements a few specific methods
mock_provider! {
    MyMockProvider,

    // Mock the get_block_with_txs method
    fn get_block_with_txs: (block_id) => {
        println!("Mock get_block_with_txs called");
        Ok(MaybePreConfirmedBlockWithTxs::PreConfirmedBlock(PreConfirmedBlockWithTxs {
           block_number: 0,
            transactions: vec![],
            timestamp: 1234567890,
            l1_gas_price: ResourcePrice { price_in_fri: Felt::from(100u32), price_in_wei: Felt::from(200u32) },
            l1_data_gas_price: ResourcePrice { price_in_fri: Felt::from(50u32), price_in_wei: Felt::from(75u32) },
            l2_gas_price: ResourcePrice { price_in_fri: Felt::from(25u32), price_in_wei: Felt::from(30u32) },
            sequencer_address: Felt::from(123u32),
            starknet_version: "0.13.0".to_string(),
            l1_da_mode: L1DataAvailabilityMode::Calldata,
        }))
    },

    // Mock the get_storage_at method using custom parameter names
    fn get_storage_at: (addr, storage_key, block) => {
        println!("Mock get_storage_at called with custom parameter names:");
        println!("  addr: {}", addr.as_ref());
        println!("  storage_key: {}", storage_key.as_ref());
        println!("  block called");

        // Return a mock storage value using custom parameter names
        Ok(Felt::from(999u32))
    },

    // Mock the chain_id method
    fn chain_id: () => {
        println!("Mock chain_id called");
        Ok(Felt::from(1u32)) // Return mock chain ID
    },

    // Mock the block_number method
    fn block_number: () => {
        println!("Mock block_number called");
        Ok(12345u64) // Return mock block number
    }
}

// Create another mock provider with different implementations and custom parameter names
mock_provider! {
    DifferentMockProvider,

    // This provider only implements chain_id differently
    fn chain_id: () => {
        println!("Different mock chain_id called");
        Ok(Felt::from(999u32)) // Different chain ID
    },

    // And a different block_number
    fn block_number: () => {
        println!("Different mock block_number called");
        Ok(54321u64) // Different block number
    },

    // Example with very descriptive custom parameter names
    fn get_nonce: (at_block_identifier, for_account_address) => {
        println!("Getting nonce for account: {}", for_account_address.as_ref());
        Ok(Felt::from(42u32))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Mock Provider Example");
    println!("========================\n");

    // Create instances of our mock providers
    let mock_provider_1 = MyMockProvider::new();
    let mock_provider_2 = DifferentMockProvider::new();

    println!("1. Testing MyMockProvider:");
    println!("--------------------------");

    // Test implemented methods
    println!("📋 Testing implemented methods:");

    let chain_id = mock_provider_1.chain_id().await?;
    println!("✅ Chain ID: {}", chain_id);

    let block_number = mock_provider_1.block_number().await?;
    println!("✅ Block number: {}", block_number);

    let storage_value = mock_provider_1
        .get_storage_at(Felt::from(0x123u32), Felt::from(0x456u32), BlockId::Tag(BlockTag::Latest))
        .await?;
    println!("✅ Storage value: {}", storage_value);

    let block = mock_provider_1.get_block_with_txs(BlockId::Tag(BlockTag::Latest)).await?;
    println!("✅ Block retrieved: {:?}", block);

    println!("\n❌ Testing unimplemented method:");

    // Test unimplemented method (this will panic with unimplemented!)
    println!("Attempting to call spec_version() - this should panic...");

    // Catch the panic to demonstrate the behavior
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { mock_provider_1.spec_version().await })
    }));

    if result.is_err() {
        println!("✅ Unimplemented method correctly panicked!");
    }

    println!("\n2. Testing DifferentMockProvider:");
    println!("---------------------------------");

    let chain_id_2 = mock_provider_2.chain_id().await?;
    println!("✅ Chain ID: {}", chain_id_2);

    let block_number_2 = mock_provider_2.block_number().await?;
    println!("✅ Block number: {}", block_number_2);

    println!("\n3. Demonstrating different behaviors:");
    println!("------------------------------------");
    println!("MyMockProvider chain_id: {}", mock_provider_1.chain_id().await?);
    println!("DifferentMockProvider chain_id: {}", mock_provider_2.chain_id().await?);
    println!("MyMockProvider block_number: {}", mock_provider_1.block_number().await?);
    println!("DifferentMockProvider block_number: {}", mock_provider_2.block_number().await?);

    println!("\n4. Testing custom parameter names:");
    println!("----------------------------------");
    let nonce =
        mock_provider_2.get_nonce(BlockId::Tag(BlockTag::Latest), Felt::from(0x1234u32)).await?;
    println!("✅ Nonce from custom params: {}", nonce);

    println!("\n✨ Example completed successfully!");
    println!("\nKey takeaways:");
    println!("- The mock_provider! macro generates a full Provider implementation");
    println!("- You only need to implement the methods you care about for testing");
    println!("- You can use custom parameter names instead of the exact trait names");
    println!("- Unimplemented methods will panic with a clear error message");
    println!("- Each mock provider can have different implementations");
    println!("- The generated structs implement Default and Debug traits");

    Ok(())
}
