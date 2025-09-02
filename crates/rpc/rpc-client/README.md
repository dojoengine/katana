# Katana RPC Client

A type-safe Rust client for interacting with Katana's Starknet JSON-RPC API.

## Features

- Complete implementation of Starknet JSON-RPC v0.9.0 specification
- Type-safe wrapper around all RPC methods
- First-class error handling for Starknet-specific errors
- Support for read, write, and trace APIs
- Async/await interface using `tokio`

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
katana-rpc-client = { path = "../path/to/katana/crates/rpc/rpc-client" }
```

### Basic Example

```rust
use jsonrpsee::http_client::HttpClientBuilder;
use katana_primitives::block::BlockIdOrTag;
use katana_rpc_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to a Katana instance
    let http_client = HttpClientBuilder::default()
        .build("http://localhost:5050")?;
    
    let client = Client::new(http_client);
    
    // Get the chain ID
    let chain_id = client.chain_id().await?;
    println!("Chain ID: {:#x}", chain_id);
    
    // Get the latest block
    let block = client
        .get_block_with_tx_hashes(BlockIdOrTag::Tag(BlockTag::Latest))
        .await?;
    
    Ok(())
}
```

### Error Handling

The client provides rich error handling with first-class support for Starknet-specific errors:

```rust
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_client::{Client, Error};

match client.get_transaction_by_hash(tx_hash).await {
    Ok(tx) => {
        println!("Transaction found: {:?}", tx);
    }
    Err(Error::Starknet(StarknetApiError::TxnHashNotFound)) => {
        println!("Transaction not found");
    }
    Err(Error::Starknet(e)) => {
        println!("Starknet error: {}", e.message());
    }
    Err(Error::Client(e)) => {
        println!("Client/transport error: {}", e);
    }
}
```

## API Methods

### Read API

- `spec_version()` - Get the RPC specification version
- `get_block_with_tx_hashes()` - Get block with transaction hashes
- `get_block_with_txs()` - Get block with full transactions
- `get_block_with_receipts()` - Get block with transactions and receipts
- `get_state_update()` - Get state update for a block
- `get_storage_at()` - Get storage value at a specific key
- `get_transaction_status()` - Get transaction status
- `get_transaction_by_hash()` - Get transaction by hash
- `get_transaction_by_block_id_and_index()` - Get transaction by block and index
- `get_transaction_receipt()` - Get transaction receipt
- `get_class()` - Get contract class definition
- `get_class_hash_at()` - Get class hash at contract address
- `get_class_at()` - Get class at contract address
- `get_block_transaction_count()` - Get number of transactions in a block
- `call()` - Call a contract function without creating a transaction
- `estimate_fee()` - Estimate transaction fees
- `estimate_message_fee()` - Estimate L1->L2 message fee
- `block_number()` - Get latest block number
- `block_hash_and_number()` - Get latest block hash and number
- `chain_id()` - Get chain ID
- `syncing()` - Get sync status
- `get_events()` - Query events with filters
- `get_nonce()` - Get account nonce
- `get_storage_proof()` - Get merkle proofs for storage

### Write API

- `add_invoke_transaction()` - Submit an invoke transaction
- `add_declare_transaction()` - Submit a declare transaction
- `add_deploy_account_transaction()` - Submit a deploy account transaction

### Trace API

- `trace_transaction()` - Get execution trace for a transaction
- `simulate_transactions()` - Simulate transactions without executing
- `trace_block_transactions()` - Get traces for all transactions in a block

## Advanced Usage

### Working with Events

```rust
use katana_rpc_types::event::{EventFilterWithPage, EventFilter, ResultPageRequest};

let filter = EventFilterWithPage {
    event_filter: EventFilter {
        from_block: Some(BlockIdOrTag::Number(0)),
        to_block: Some(BlockIdOrTag::Tag(BlockTag::Latest)),
        address: Some(contract_address),
        keys: None,
    },
    page_request: ResultPageRequest {
        continuation_token: None,
        limit: Some(100),
    },
};

let events = client.get_events(filter).await?;
for event in events.events {
    println!("Event: {:?}", event);
}
```

### Simulating Transactions

```rust
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::SimulationFlag;

let transactions = vec![/* your transactions */];
let flags = vec![SimulationFlag::SkipValidate];

let simulations = client
    .simulate_transactions(
        BlockIdOrTag::Tag(BlockTag::Latest),
        transactions,
        flags,
    )
    .await?;
```

### Accessing the Inner HTTP Client

For advanced use cases, you can access the underlying HTTP client:

```rust
let inner_client = client.inner();
// Use inner_client for custom RPC calls
```

## Running Tests

Integration tests require a running Katana instance:

```bash
# Start Katana in one terminal
katana

# Run tests in another terminal
cargo test --test integration -- --ignored
```

## License

See the main Katana repository for license information.