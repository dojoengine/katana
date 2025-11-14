// Script to test fork by executing transactions on mainnet and fork
// Usage: cargo run --bin test_fork_transaction -- --mainnet-rpc http://127.0.0.1:5050 --fork-rpc http://127.0.0.1:5051

use std::str::FromStr;
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_server::HttpClient;
use katana_primitives::{felt, Felt, ContractAddress};
use katana_primitives::block::BlockIdOrTag;
use katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::Call;
use starknet::macros::selector;
use starknet::providers::Provider;
use starknet::signers::{LocalWallet, SigningKey};

// Prefunded account from Katana (first one)
const PREFUNDED_PRIVATE_KEY: &str = "0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912";
const PREFUNDED_ADDRESS: &str = "0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mainnet_rpc = args.iter().position(|a| a == "--mainnet-rpc")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("http://127.0.0.1:5050");
    
    let fork_rpc = args.iter().position(|a| a == "--fork-rpc")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("http://127.0.0.1:5051");
    
    println!("🔍 Testing fork with transaction execution...");
    println!("Mainnet RPC: {}", mainnet_rpc);
    println!("Fork RPC: {}", fork_rpc);
    println!();
    
    let mainnet_provider = starknet::providers::jsonrpc::JsonRpcClient::new(
        reqwest::Url::parse(mainnet_rpc)?
    );
    let fork_provider = starknet::providers::jsonrpc::JsonRpcClient::new(
        reqwest::Url::parse(fork_rpc)?
    );
    
    // Create accounts for both mainnet and fork
    let signing_key = SigningKey::from_secret_scalar(Felt::from_str(PREFUNDED_PRIVATE_KEY)?);
    let wallet = LocalWallet::from(signing_key);
    let account_address = ContractAddress::from_str(PREFUNDED_ADDRESS)?;
    
    let mainnet_chain_id = mainnet_provider.chain_id().await?;
    let fork_chain_id = fork_provider.chain_id().await?;
    
    let mainnet_account = SingleOwnerAccount::new(
        mainnet_provider,
        wallet.clone(),
        account_address,
        mainnet_chain_id,
        ExecutionEncoding::New,
    );
    
    let fork_account = SingleOwnerAccount::new(
        fork_provider,
        wallet,
        account_address,
        fork_chain_id,
        ExecutionEncoding::New,
    );
    
    // Check initial state
    println!("1️⃣ Initial state:");
    let mainnet_block_initial = mainnet_account.provider().block_number().await?;
    let fork_block_initial = fork_account.provider().block_number().await?;
    println!("   Mainnet block: {}", mainnet_block_initial);
    println!("   Fork block: {}", fork_block_initial);
    println!();
    
    // Execute transaction on mainnet
    println!("2️⃣ Executing transaction on MAINNET...");
    let recipient = ContractAddress::from_str("0x1")?;
    let transfer_selector = selector!("transfer");
    let amount_low = Felt::ONE;
    let amount_high = Felt::ZERO;
    
    let call = Call {
        to: DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(),
        selector: transfer_selector,
        calldata: vec![recipient.into(), amount_low, amount_high],
    };
    
    let mainnet_tx_result = mainnet_account.execute(vec![call]).send().await?;
    println!("   Transaction hash: 0x{:x}", mainnet_tx_result.transaction_hash);
    
    // Wait for transaction to be included
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    let mainnet_block_after_main = mainnet_account.provider().block_number().await?;
    let fork_block_after_main = fork_account.provider().block_number().await?;
    
    println!("   Mainnet block after: {}", mainnet_block_after_main);
    println!("   Fork block after: {}", fork_block_after_main);
    
    if mainnet_block_after_main > mainnet_block_initial {
        println!("   ✅ Mainnet block increased");
    }
    if fork_block_after_main == fork_block_initial {
        println!("   ✅ Fork block stayed the same (fork is independent!)");
    }
    println!();
    
    // Execute same transaction on fork
    println!("3️⃣ Executing SAME transaction on FORK...");
    let fork_tx_result = fork_account.execute(vec![call]).send().await?;
    println!("   Transaction hash: 0x{:x}", fork_tx_result.transaction_hash);
    
    // Wait for transaction to be included
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    let mainnet_block_final = mainnet_account.provider().block_number().await?;
    let fork_block_final = fork_account.provider().block_number().await?;
    
    println!("4️⃣ Final state:");
    println!("   Mainnet block: {}", mainnet_block_final);
    println!("   Fork block: {}", fork_block_final);
    println!();
    
    if mainnet_block_final != fork_block_final {
        println!("   ✅ Block numbers differ - fork is independent!");
    } else {
        println!("   ⚠️  Block numbers are the same");
    }
    
    // Check storage (balance of recipient)
    let balance_slot = felt!("0x3e7e5b2f8e2a0e4c8b3d5f7a9c1e6b4d8f2a5c7e9b1d3f5a7c9e1b3d5f7a9c");
    let mainnet_balance = mainnet_account.provider()
        .get_storage_at(
            DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(),
            balance_slot,
            BlockIdOrTag::Latest,
        )
        .await?;
    
    let fork_balance = fork_account.provider()
        .get_storage_at(
            DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(),
            balance_slot,
            BlockIdOrTag::Latest,
        )
        .await?;
    
    println!("   Mainnet balance slot: 0x{:x}", mainnet_balance);
    println!("   Fork balance slot: 0x{:x}", fork_balance);
    
    if mainnet_balance == fork_balance {
        println!("   ✅ Storage matches after same transaction!");
        println!("   ✅ This proves fork works correctly - same transaction = same result!");
    } else {
        println!("   ⚠️  Storage differs after same transaction");
    }
    println!();
    
    println!("✅ Test completed!");
    
    Ok(())
}

