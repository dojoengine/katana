# Katana Test Utils Macro

This crate provides procedural macros for creating mock implementations of traits for testing purposes.

## Overview

The `mock_provider!` macro allows you to create mock implementations of the Starknet `Provider` trait with minimal boilerplate. You only need to implement the methods you care about for your specific tests - all other methods will automatically use `unimplemented!()` as a placeholder.

## Usage

### Basic Example

```rust
use katana_utils::mock_provider;
use starknet::core::types::{BlockId, Felt};
use starknet::providers::Provider;

mock_provider! {
    MyMockProvider,

    fn get_storage_at: (contract_address, key, block_id) => {
        // Your custom implementation here
        Ok(Felt::from(42u32))
    },

    fn chain_id: () => {
        Ok(Felt::from(1u32))
    }
}

#[tokio::test]
async fn test_my_mock_provider() {
    let provider = MyMockProvider::new();

    // Test implemented methods
    let storage = provider.get_storage_at(
        Felt::from(1u32),
        Felt::from(2u32),
        BlockId::Tag(BlockTag::Latest)
    ).await.unwrap();
    assert_eq!(storage, Felt::from(42u32));

    let chain_id = provider.chain_id().await.unwrap();
    assert_eq!(chain_id, Felt::from(1u32));

    // Unimplemented methods will panic with a clear error message
    // provider.spec_version().await; // This would panic!
}
```

### Multiple Mock Providers

You can create multiple mock providers with different implementations:

```rust
mock_provider! {
    TestnetMockProvider,

    fn chain_id: () => {
        Ok(Felt::from(1u32))
    },

    fn block_number: () => {
        Ok(12345u64)
    }
}

mock_provider! {
    LocalMockProvider,

    fn chain_id: () => {
        Ok(Felt::from(1536727068981429685321u128))
    },

    fn block_number: () => {
        Ok(1u64)
    }
}
```

## Syntax

The macro follows this syntax:

```rust
mock_provider! {
    StructName,

    fn method_name: (param1, param2, ...) => {
        // Method implementation
        // Must return the appropriate Result type
    },

    fn another_method: (param) => {
        // Another implementation
    }
}
```

### Key Points

1. **Struct Name**: The first parameter is the name of the struct to generate
2. **fn Keyword**: Each method must be prefixed with the `fn` keyword
3. **Method Signature**: You only need to specify the parameter names, not their types or the full signature
4. **Implementation**: The body should return the appropriate `Result` type for the method
5. **Async Methods**: All Provider methods are async, so your implementations should return the result directly (not wrapped in a Future)

## Generated Code

The macro generates:

1. A struct with the specified name
2. `Debug`, `Clone`, `Default` implementations
3. A `new()` constructor method
4. A complete `Provider` trait implementation with:
   - Your custom implementations for specified methods
   - `unimplemented!()` for all other methods with descriptive error messages

## Supported Methods

The macro supports all methods from the `starknet::providers::Provider` trait, including:

- `spec_version`
- `get_block_with_tx_hashes`
- `get_block_with_txs`
- `get_block_with_receipts`
- `get_state_update`
- `get_storage_at`
- `get_messages_status`
- `get_transaction_status`
- `get_transaction_by_hash`
- `get_transaction_by_block_id_and_index`
- `get_transaction_receipt`
- `get_class`
- `get_class_hash_at`
- `get_class_at`
- `get_block_transaction_count`
- `call`
- `estimate_fee`
- `estimate_message_fee`
- `block_number`
- `block_hash_and_number`
- `chain_id`
- `syncing`
- `get_events`
- `get_nonce`
- `get_storage_proof`
- `add_invoke_transaction`
- `add_declare_transaction`
- `add_deploy_account_transaction`
- `trace_transaction`
- `simulate_transactions`
- `trace_block_transactions`
- `batch_requests`
- `estimate_fee_single`
- `simulate_transaction`

## Error Handling

Methods that are not implemented will panic with a descriptive error message:

```
Method get_transaction_by_hash not implemented in mock
```

This makes it easy to identify which methods your tests are trying to use that you haven't implemented yet.

## Testing

To test the macro itself:

```bash
cargo test -p katana-test-utils-macro
```

To run the example:

```bash
cargo run --example mock_provider_example
```
