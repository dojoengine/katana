use std::path::PathBuf;

use anyhow::Result;
use cairo_vm::types::layout_name::LayoutName;
use katana_chain_spec::rollup::{self, ChainConfigDir};
use katana_chain_spec::ChainSpec;
use katana_messaging::MessagingConfig;
use katana_sequencer_node::config::db::DbConfig;
use katana_sequencer_node::config::Config;
use katana_sequencer_node::{LaunchedNode, Node};
use katana_primitives::block::BlockNumber;
use katana_primitives::{address, Felt};
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::{DbProviderFactory, ProviderFactory};

#[tokio::main]
async fn main() {
    let node = node().await;

    let provider = node.node().backend().storage.provider();
    let url = format!("http://{}", node.rpc().addr());

    let latest_block = provider.latest_number().expect("failed to get latest block number");
    println!("Processing {latest_block} blocks");

    for block in 0..latest_block {
        println!("Processing block {block}");
        run_snos(block, &url).await.expect("Failed to run snos for block {i}");
    }

    println!("Finished processing {latest_block} blocks")
}

async fn run_snos(block: BlockNumber, rpc_url: &str) -> Result<()> {
    const DEFAULT_COMPILED_OS: &[u8] = include_bytes!("../snos/build/os_latest.json");
    const LAYOUT: LayoutName = LayoutName::all_cairo;

    let (.., output) = snos::prove_block(DEFAULT_COMPILED_OS, block, rpc_url, LAYOUT, true).await?;

    if block == 0 {
        assert_eq!(output.prev_block_number, Felt::MAX);
        assert_eq!(output.new_block_number, Felt::ZERO);
    } else {
        assert_eq!(output.prev_block_number, Felt::from(block - 1));
        assert_eq!(output.new_block_number, Felt::from(block));
    }

    Ok(())
}

async fn node() -> LaunchedNode<DbProviderFactory> {
    // These paths only work if you run from the root of the repository:
    //
    // cargo run -p snos-integration-tests
    const TEST_CHAIN_CONFIG: &str = "tests/fixtures/test-chain";
    const TEST_DB_DIR: &str = "tests/fixtures/katana_db";

    let config_dir = ChainConfigDir::open(TEST_CHAIN_CONFIG).unwrap();
    let mut chain = rollup::read(&config_dir).expect("failed to read chain config");
    chain.genesis.sequencer_address = address!("0x1"); // this is so stupid

    let messaging = MessagingConfig::from_chain_spec(&chain);
    let chain = ChainSpec::Rollup(chain);

    let config = Config {
        chain: chain.into(),
        messaging: Some(messaging),
        db: DbConfig { dir: Some(PathBuf::from(TEST_DB_DIR)) },
        ..Default::default()
    };

    Node::build(config)
        .expect("failed to build node")
        .launch()
        .await
        .expect("failed to launch node")
}
