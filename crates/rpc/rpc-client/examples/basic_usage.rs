use jsonrpsee::http_client::HttpClientBuilder;
use katana_primitives::block::BlockIdOrTag;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_client::{Client, Error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create an HTTP client connected to a local Katana instance
    let http_client = HttpClientBuilder::default().build("http://localhost:5050")?;

    // Create the Starknet client wrapper
    let client = Client::new(http_client);

    // Get the spec version
    match client.spec_version().await {
        Ok(version) => println!("Starknet RPC spec version: {}", version),
        Err(Error::Starknet(e)) => {
            // Handle Starknet-specific errors
            println!("Starknet API error: {}", e.message());
            if let Some(data) = e.data() {
                println!("Error data: {:?}", data);
            }
        }
        Err(Error::Client(e)) => {
            // Handle transport/client errors
            println!("Client error: {}", e);
        }
    }

    // Get the chain ID
    let chain_id = client.chain_id().await?;
    println!("Chain ID: {:#x}", chain_id);

    // Get the latest block number
    let block_number = client.block_number().await?;
    println!("Latest block number: {}", block_number.block_number);

    // Example of handling specific Starknet errors
    let invalid_block_id = BlockIdOrTag::Number(999999999);
    match client.get_block_with_tx_hashes(invalid_block_id).await {
        Ok(block) => match block {
            katana_rpc_types::block::MaybePreConfirmedBlockWithTxHashes::Block(block) => {
                println!("Block hash: {:#x}", block.block_hash);
            }
            katana_rpc_types::block::MaybePreConfirmedBlockWithTxHashes::PreConfirmed(block) => {
                println!("Pre-confirmed block with {} transactions", block.transactions.len());
            }
        },
        Err(Error::Starknet(StarknetApiError::BlockNotFound)) => {
            println!("Block not found (expected for invalid block number)");
        }
        Err(e) => {
            println!("Unexpected error: {}", e);
        }
    }

    // Get the latest block with transaction hashes
    let block = client
        .get_block_with_tx_hashes(BlockIdOrTag::Tag(katana_primitives::block::BlockTag::Latest))
        .await?;
    match block {
        katana_rpc_types::block::MaybePreConfirmedBlockWithTxHashes::Block(block) => {
            println!("Block hash: {:#x}", block.block_hash);
            println!("Number of transactions: {}", block.transactions.len());
        }
        katana_rpc_types::block::MaybePreConfirmedBlockWithTxHashes::PreConfirmed(block) => {
            println!("Pre-confirmed block with {} transactions", block.transactions.len());
        }
    }

    // Check sync status
    let sync_status = client.syncing().await?;
    match sync_status {
        katana_rpc_types::SyncingStatus::NotSyncing => {
            println!("Node is not syncing");
        }
        katana_rpc_types::SyncingStatus::Syncing(status) => {
            println!(
                "Node is syncing: current block = {}, highest block = {}",
                status.current_block_num, status.highest_block_num
            );
        }
    }

    // Example of checking for specific errors using pattern matching
    let invalid_tx_hash = katana_primitives::transaction::TxHash::from_hex("0x123")?;
    match client.get_transaction_by_hash(invalid_tx_hash).await {
        Ok(tx) => println!("Found transaction: {:?}", tx.transaction_hash),
        Err(Error::Starknet(StarknetApiError::TxnHashNotFound)) => {
            println!("Transaction not found (expected)");
        }
        Err(e) => println!("Other error: {}", e),
    }

    Ok(())
}
