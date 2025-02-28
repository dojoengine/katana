use std::path::PathBuf;

use anyhow::Result;
use cairo_vm::types::layout_name::LayoutName;
use katana_chain_spec::rollup::{self, ChainConfigDir};
use katana_chain_spec::ChainSpec;
use katana_node::config::db::DbConfig;
use katana_node::config::Config;
use katana_node::{LaunchedNode, Node};
use katana_primitives::block::BlockNumber;
use katana_primitives::Felt;
use katana_provider::traits::block::BlockNumberProvider;

#[tokio::main]
async fn main() {
    let node = node().await.expect("failed to start node");
    let provider = node.node().backend().blockchain.provider();
    let url = format!("http://{}", node.rpc().addr());

    let latest_block = provider.latest_number().expect("failed to get latest block number");
    println!("Proving blocks from 0 to {latest_block}");

    for block in 0..latest_block {
        println!("Processing block {block}");
        run_snos(block, &url).await.expect("Failed to run snos for block {i}");
    }
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

async fn node() -> Result<LaunchedNode> {
    let chain = rollup::read(&ChainConfigDir::open("../fixtures/test-chain")?)?;
    let chain = ChainSpec::Rollup(chain);

    let db = DbConfig { dir: Some(PathBuf::from(".")) };
    let config = Config { chain: chain.into(), db, ..Default::default() };

    Node::build(config).await?.launch().await
}
