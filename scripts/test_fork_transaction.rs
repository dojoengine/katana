//! Script that automatically sends the same transaction to mainnet & fork nodes.
//! Set env vars if needed:
//!   MAINNET_RPC=http://127.0.0.1:5050
//!   FORK_RPC=http://127.0.0.1:5051
//!   RECIPIENT_ADDRESS=0x2
//!   TRANSFER_AMOUNT=0x1
//! This test requires mainnet to have at least one block

use std::{fmt::Display, sync::Arc, time::Duration};

use starknet::{
    accounts::{Account, ExecutionEncoding, SingleOwnerAccount},
    core::{
        types::{
            BlockId, BlockTag, Call, ConfirmedBlockId, ContractStorageKeys, Felt, StorageProof,
        },
        utils::get_storage_var_address,
    },
    macros::selector,
    providers::{
        jsonrpc::{HttpTransport, JsonRpcClient},
        Provider,
    },
    signers::{LocalWallet, SigningKey},
};
use tokio::time::sleep;
use url::Url;

const PREFUNDED_PRIVATE_KEY: &str =
    "0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912";
const PREFUNDED_ACCOUNT_ADDRESS: &str =
    "0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec";
const DEFAULT_ETH_FEE_TOKEN_ADDRESS: &str =
    "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
const DEFAULT_RECIPIENT_ADDRESS: &str = "0x2";
const DEFAULT_TRANSFER_AMOUNT: &str = "0x1";
type RpcProvider = Arc<JsonRpcClient<HttpTransport>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mainnet_rpc =
        std::env::var("MAINNET_RPC").unwrap_or_else(|_| "http://127.0.0.1:5050".to_string());
    let fork_rpc =
        std::env::var("FORK_RPC").unwrap_or_else(|_| "http://127.0.0.1:5051".to_string());
    let recipient_hex = std::env::var("RECIPIENT_ADDRESS")
        .unwrap_or_else(|_| DEFAULT_RECIPIENT_ADDRESS.to_string());
    let amount_hex =
        std::env::var("TRANSFER_AMOUNT").unwrap_or_else(|_| DEFAULT_TRANSFER_AMOUNT.to_string());

    section("Configuration");
    info("Mainnet RPC", &mainnet_rpc);
    info("Fork RPC", &fork_rpc);
    info("Recipient", &recipient_hex);
    info("Transfer amount (low felt)", &amount_hex);

    let mainnet_provider = build_provider(&mainnet_rpc)?;
    let fork_provider = build_provider(&fork_rpc)?;

    let account_address = Felt::from_hex(PREFUNDED_ACCOUNT_ADDRESS)?;
    let recipient = Felt::from_hex(&recipient_hex)?;
    let amount_low = Felt::from_hex(&amount_hex)?;
    let amount_high = Felt::ZERO;
    let fee_token = Felt::from_hex(DEFAULT_ETH_FEE_TOKEN_ADDRESS)?;

    let private_key = Felt::from_hex(PREFUNDED_PRIVATE_KEY)?;
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(private_key));

    let mainnet_account =
        build_account(mainnet_provider.clone(), signer.clone(), account_address).await?;
    let fork_account = build_account(fork_provider.clone(), signer, account_address).await?;

    section("Prerequisites");
    info(
        "CallTest contract",
        "expected to be declared & deployed on both nodes",
    );
    info("Helper", "run `deploy_call_test` binary if setup is missing");

    let storage_base = get_storage_var_address("ERC20_balances", &[recipient])?;

    // ---------------------------------------------------------------------
    section("Initial state");
    let mainnet_block_initial = mainnet_provider.block_number().await?;
    let fork_block_initial = fork_provider.block_number().await?;
    let (mainnet_bal_low_before, mainnet_bal_high_before) =
        fetch_balance(&mainnet_provider, fee_token, storage_base).await?;
    let (fork_bal_low_before, fork_bal_high_before) =
        fetch_balance(&fork_provider, fee_token, storage_base).await?;

    print_block_info("Mainnet", mainnet_block_initial);
    print_block_info("Fork   ", fork_block_initial);
    print_balance("Mainnet", (mainnet_bal_low_before, mainnet_bal_high_before), false);
    print_balance("Fork   ", (fork_bal_low_before, fork_bal_high_before), false);
    println!();

    // ---------------------------------------------------------------------
    section("Transaction on mainnet");
    let mainnet_call = build_transfer_call(fee_token, recipient, amount_low, amount_high);
    let mainnet_hash =
        mainnet_account.execute_v3(vec![mainnet_call]).send().await?.transaction_hash;
    info("Mainnet tx hash", format!("{mainnet_hash:#x}"));
    sleep(Duration::from_secs(2)).await;

    let mainnet_block_after = mainnet_provider.block_number().await?;
    let fork_block_after_main = fork_provider.block_number().await?;
    info(
        "Blocks after mainnet tx",
        format!("mainnet {mainnet_block_after}, fork {fork_block_after_main}"),
    );
    if mainnet_block_after > mainnet_block_initial {
        info("Mainnet inclusion", "block advanced");
    }
    if fork_block_after_main == fork_block_initial {
        info("Fork isolation", "block unchanged");
    }

    // ---------------------------------------------------------------------
    section("Transaction on fork");
    let fork_call = build_transfer_call(fee_token, recipient, amount_low, amount_high);
    let fork_hash = fork_account.execute_v3(vec![fork_call]).send().await?.transaction_hash;
    info("Fork tx hash", format!("{fork_hash:#x}"));
    sleep(Duration::from_secs(2)).await;

    let mainnet_block_final = mainnet_provider.block_number().await?;
    let fork_block_final = fork_provider.block_number().await?;
    info(
        "Blocks after fork tx",
        format!("mainnet {mainnet_block_final}, fork {fork_block_final}"),
    );

    // ---------------------------------------------------------------------
    section("Final balances");
    let mainnet_balance_final = fetch_balance(&mainnet_provider, fee_token, storage_base).await?;
    let fork_balance_final = fetch_balance(&fork_provider, fee_token, storage_base).await?;

    print_balance("Mainnet", mainnet_balance_final, true);
    print_balance("Fork   ", fork_balance_final, true);
    println!();

    if mainnet_balance_final == fork_balance_final {
        info("Storage comparison", "matches on both nodes");
    } else {
        info("Storage comparison", "DIFFERS (check fork behaviour)");
    }

    // ---------------------------------------------------------------------
    section("getStorageProof comparison");
    let mainnet_roots = fetch_global_roots(&mainnet_provider, mainnet_block_final).await?;
    let fork_roots = fetch_global_roots(&fork_provider, fork_block_final).await?;

    print_global_roots("Mainnet", &mainnet_roots);
    print_global_roots("Fork   ", &fork_roots);

    if mainnet_roots.global_roots.contracts_tree_root == fork_roots.global_roots.contracts_tree_root
    {
        info("Contracts root", "matches");
    } else {
        info("Contracts root", "differs");
    }

    if mainnet_roots.global_roots.classes_tree_root == fork_roots.global_roots.classes_tree_root {
        info("Classes root", "matches");
    } else {
        info("Classes root", "differs");
    }

    if mainnet_roots.global_roots.block_hash == fork_roots.global_roots.block_hash {
        info("Block hashes", "identical");
    } else {
        info("Block hashes", "differ (expected)");
    }

    section("Summary");
    println!("✅ Test finished");
    Ok(())
}

fn build_provider(rpc: &str) -> Result<RpcProvider, Box<dyn std::error::Error>> {
    let url = Url::parse(rpc)?;
    Ok(Arc::new(JsonRpcClient::new(HttpTransport::new(url))))
}

async fn build_account(
    provider: RpcProvider,
    wallet: LocalWallet,
    address: Felt,
) -> Result<SingleOwnerAccount<RpcProvider, LocalWallet>, Box<dyn std::error::Error>> {
    let chain_id = provider.chain_id().await?;
    Ok(SingleOwnerAccount::new(provider, wallet, address, chain_id, ExecutionEncoding::New))
}

fn build_transfer_call(
    contract: Felt,
    recipient: Felt,
    amount_low: Felt,
    amount_high: Felt,
) -> Call {
    Call {
        to: contract,
        selector: selector!("transfer"),
        calldata: vec![recipient, amount_low, amount_high],
    }
}

async fn fetch_balance(
    provider: &RpcProvider,
    contract: Felt,
    base_slot: Felt,
) -> Result<(Felt, Felt), Box<dyn std::error::Error>> {
    let low = provider.get_storage_at(contract, base_slot, BlockId::Tag(BlockTag::Latest)).await?;
    let high = provider
        .get_storage_at(contract, base_slot + Felt::ONE, BlockId::Tag(BlockTag::Latest))
        .await?;
    Ok((low, high))
}

fn print_block_info(label: &str, block: u64) {
    info(label, format!("block {block}"));
}

fn print_balance(label: &str, (low, high): (Felt, Felt), final_state: bool) {
    let prefix = if final_state { "final" } else { "initial" };
    println!("   {label} balance ({prefix}) -> low: {:#x}, high: {:#x}", low, high);
}

async fn fetch_global_roots(
    provider: &RpcProvider,
    block_number: u64,
) -> Result<StorageProof, Box<dyn std::error::Error>> {
    let empty_hashes: [Felt; 0] = [];
    let empty_addresses: [Felt; 0] = [];
    let empty_storage: [ContractStorageKeys; 0] = [];
    Ok(provider
        .get_storage_proof(
            ConfirmedBlockId::Number(block_number),
            &empty_hashes,
            &empty_addresses,
            &empty_storage,
        )
        .await?)
}

fn print_global_roots(label: &str, proof: &StorageProof) {
    info(
        &format!("{label} contracts root"),
        format!("{:#x}", proof.global_roots.contracts_tree_root),
    );
    info(&format!("{label} classes root"), format!("{:#x}", proof.global_roots.classes_tree_root));
    info(&format!("{label} block hash"), format!("{:#x}", proof.global_roots.block_hash));
}

fn section(title: &str) {
    println!("\n=== {title} ===");
}

fn info(label: &str, value: impl Display) {
    println!("{label:>24}: {value}");
}
