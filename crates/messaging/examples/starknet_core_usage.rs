//! Example demonstrating how to use the Starknet Core Contract bindings.
//!
//! Run with:
//! ```bash
//! cargo run --example starknet_core_usage
//! ```

use anyhow::Result;
use katana_messaging::starknet_core::{StarknetCore, STARKNET_CORE_CONTRACT_ADDRESS};

#[tokio::main]
async fn main() -> Result<()> {
    // You'll need to set your Ethereum RPC URL
    let rpc_url = std::env::var("ETH_RPC_URL")
        .unwrap_or_else(|_| "https://eth-mainnet.g.alchemy.com/v2/demo".to_string());

    println!("Connecting to Ethereum RPC: {}", rpc_url);
    println!("Starknet Core Contract: {}", STARKNET_CORE_CONTRACT_ADDRESS);

    // Create a client for the official Starknet mainnet contract
    let client = StarknetCore::new_http_mainnet(&rpc_url).await?;

    // Example: Fetch recent state updates
    // Note: Adjust these block numbers based on current Ethereum block height
    let from_block = 18000000;
    let to_block = from_block + 100;

    println!("\nFetching LogStateUpdate events from block {} to {}...", from_block, to_block);

    match client.fetch_decoded_state_updates(from_block, to_block).await {
        Ok(state_updates) => {
            println!("Found {} state update(s)", state_updates.len());

            for (i, update) in state_updates.iter().enumerate() {
                println!("\n--- State Update #{} ---", i + 1);
                println!("Global Root: {:#x}", update.globalRoot);
                println!("Block Number: {}", update.blockNumber);
                println!("Block Hash: {:#x}", update.blockHash);
            }
        }
        Err(e) => {
            eprintln!("Error fetching state updates: {}", e);
            eprintln!("\nNote: You may need to:");
            eprintln!("  1. Set ETH_RPC_URL environment variable to a valid Ethereum RPC endpoint");
            eprintln!("  2. Adjust the block range to match actual Starknet state update blocks");
        }
    }

    Ok(())
}
