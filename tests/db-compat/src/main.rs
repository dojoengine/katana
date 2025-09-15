use anyhow::Result;
use katana_cli::args::Parser;
use katana_db::version::CURRENT_DB_VERSION;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, Url};

// TODO(kariy): update this test to using the Node struct to initialize Katana
#[tokio::main]
async fn main() -> Result<()> {
    println!("Testing database compatibility from version 1.6.0");
    println!("Current Katana database version: {CURRENT_DB_VERSION}");

    const TEST_DB_DIR: &str = "tests/fixtures/db/db_v1.6.0";

    let node = katana_cli::NodeArgs::parse_from(["katana", "--db-dir", TEST_DB_DIR]);
    let addr = node.rpc_config().unwrap().socket_addr();
    tokio::spawn(async move { node.execute().await });

    // Give the node some time to start up
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    let url = Url::parse(format!("http://{addr}").as_str())?;
    let client = JsonRpcClient::new(HttpTransport::new(url));

    let latest_block_number = client.block_number().await?;
    println!("Latest block number: {}", latest_block_number);
    println!("Successfully initialized Katana with v1.2.2 database!");

    Ok(())
}
