use anyhow::Result;
use katana_db::version::CURRENT_DB_VERSION;
use katana_node_bindings::Katana;
use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::{address, felt};
use katana_rpc_client::starknet::{Client as StarknetClient, Error as RpcError, StarknetApiError};

#[tokio::main]
async fn main() -> Result<()> {
    println!("Testing database compatibility from version 1.6.0");
    println!("Current Katana database version: {CURRENT_DB_VERSION}");

    const TEST_DB_DIR: &str = "tests/fixtures/db/1_6_0";

    // Get the katana binary path from the environment variable if provided (for CI)
    // Otherwise, assume it's in PATH
    let katana = if let Ok(binary_path) = std::env::var("KATANA_BIN") {
        Katana::at(binary_path)
    } else {
        Katana::new()
    };

    let instance = katana.data_dir(TEST_DB_DIR).spawn();

    // Create HTTP client for Katana's RPC
    let url = format!("http://{}", instance.rpc_addr());
    println!("Katana RPC URL: {}", url);

    // Give the node some time to fully initialize
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let client = StarknetClient::new(url.as_str().try_into().unwrap());

    // Run all RPC tests
    test_rpc_queries(&client).await;

    println!("\n=== Database Compatibility Test PASSED ===");
    println!(
        "Successfully initialized Katana with v1.6.0 database and exercised all major database \
         tables!"
    );

    Ok(())
}

async fn test_rpc_queries(client: &StarknetClient) {
    let latest_block_number = client.block_number().await.unwrap().block_number;

    // sanity check
    // there should be 27 blocks in the test database
    assert_eq!(latest_block_number, 27);

    let _ = client.get_block_with_txs(latest_block_number.into()).await.unwrap();
    let _ = client.get_block_with_tx_hashes(latest_block_number.into()).await.unwrap();
    let _ = client.get_block_transaction_count(latest_block_number.into()).await.unwrap();

    let tx_hash = felt!("0x241f540802ddd4eefc1b7513b630c30fc684b304b97eb0c03af625e2dffd12");
    let _ = client.get_transaction_by_hash(tx_hash).await.unwrap();
    let _ = client.get_transaction_receipt(tx_hash).await.unwrap();
    let _ = client.get_transaction_by_block_id_and_index(BlockIdOrTag::Number(1), 0).await.unwrap();

    let tx_hash = felt!("0x722b80b0fd8e4cd177bb565ee73843ce5ffb8cc07114207cd4399cb4e00c9ac");
    // The v1.6.0 database contains TransactionExecutionInfo serialized with an older blockifier
    // format. Since #392 (https://github.com/dojoengine/katana/pull/392), the new blockifier
    // format is incompatible so deserialization fails and the provider returns None, which
    // surfaces as TxnHashNotFound.
    match client.trace_transaction(tx_hash).await {
        Ok(_) => {}
        Err(RpcError::Starknet(StarknetApiError::TxnHashNotFound)) => {}
        Err(e) => panic!("unexpected error from trace_transaction: {e}"),
    }
    let _ = client.trace_block_transactions(ConfirmedBlockIdOrTag::Number(1)).await.unwrap();

    let class_hash = felt!("0x685ed02eefa98fe7e208aa295042e9bbad8029b0d3d6f0ba2b32546efe0a1f9");
    let _ = client.get_class(BlockIdOrTag::Latest, class_hash).await.unwrap();

    let address = address!("0x2af9427c5a277474c079a1283c880ee8a6f0f8fbf73ce969c08d88befec1bba");
    let _ = client.get_class_at(BlockIdOrTag::Latest, address).await.unwrap();
    let _ = client.get_class_hash_at(BlockIdOrTag::Latest, address).await.unwrap();
    let _ = client.get_nonce(BlockIdOrTag::Latest, address).await.unwrap();
}
