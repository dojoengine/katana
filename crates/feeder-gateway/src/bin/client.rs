use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use katana_feeder_gateway::client::{Error as GatewayError, SequencerGateway};
use katana_feeder_gateway::types::{BlockId, StateUpdate, StateUpdateWithBlock};
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::class::ClassHash;
use katana_primitives::Felt;
use tracing::error;
use url::Url;

#[derive(Debug, Parser)]
#[command(
    name = "gateway",
    version,
    about = "Interact with the Starknet gateway",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Target sequencer network.
    #[arg(global = true, long, value_enum, default_value_t = Network::Mainnet, conflicts_with = "base_url")]
    network: Network,

    /// Override the feeder gateway base URL.
    #[arg(global = true, long, value_name = "URL", conflicts_with = "network")]
    base_url: Option<String>,

    /// Optional gateway API key to bypass rate limiting.
    #[arg(global = true, long)]
    api_key: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Network {
    Mainnet,
    Sepolia,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Fetch a block.
    GetBlock(BlockCommand),
    /// Fetch state update information for a block.
    GetStateUpdate(StateUpdateCommand),
    /// Fetch a class definition by hash.
    GetClass(ClassCommand),
    /// Fetch a compiled class by hash.
    GetCompiledClass(ClassCommand),
}

#[derive(Debug, Args)]
struct BlockCommand {
    #[command(flatten)]
    block: BlockIdArgs,
}

#[derive(Debug, Args)]
struct StateUpdateCommand {
    #[command(flatten)]
    block: BlockIdArgs,

    /// Include block information alongside the state update response.
    #[arg(long)]
    with_block: bool,
}

#[derive(Debug, Args)]
struct ClassCommand {
    #[command(flatten)]
    block: BlockIdArgs,

    /// Class hash to fetch.
    #[arg(long, value_name = "FELT")]
    class_hash: ClassHash,
}

#[derive(Debug, Default, Clone, Args)]
struct BlockIdArgs {
    /// Use a specific block hash.
    #[arg(long, value_name = "HASH", conflicts_with_all = ["block_number", "latest"])]
    block_hash: Option<BlockHash>,

    /// Use a specific block number.
    #[arg(long, value_name = "NUMBER", conflicts_with_all = ["block_hash", "latest"])]
    block_number: Option<BlockNumber>,

    /// Explicitly target the latest block.
    #[arg(long, conflicts_with_all = ["block_hash", "block_number"])]
    latest: bool,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        error!(target: "katana_feeder_gateway::cli", "{err:#}");
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let mut gateway = build_gateway(&cli).context("failed to construct feeder gateway client")?;

    if let Some(api_key) = cli.api_key {
        gateway = gateway.with_api_key(api_key);
    }

    match cli.command {
        Command::GetBlock(cmd) => {
            let block_id = cmd.block.resolve()?;
            let block = gateway.get_block(block_id).await.map_err(map_gateway_error)?;
            print_debug(block);
        }
        Command::GetStateUpdate(cmd) => {
            let block_id = cmd.block.resolve()?;
            if cmd.with_block {
                let update = gateway
                    .get_state_update_with_block(block_id)
                    .await
                    .map_err(map_gateway_error)?;
                print_state_update_with_block(&update);
            } else {
                let update = gateway.get_state_update(block_id).await.map_err(map_gateway_error)?;
                print_state_update(&update);
            }
        }
        Command::GetClass(cmd) => {
            let block_id = cmd.block.resolve()?;
            let class_hash = cmd.class_hash;
            let class =
                gateway.get_class(cmd.class_hash, block_id).await.map_err(map_gateway_error)?;
            print_debug(class);
        }
        Command::GetCompiledClass(cmd) => {
            let block_id = cmd.block.resolve()?;
            let class_hash = cmd.class_hash;
            let class = gateway
                .get_compiled_class(class_hash, block_id)
                .await
                .map_err(map_gateway_error)?;
            print_debug(class);
        }
    }

    Ok(())
}

fn build_gateway(cli: &Cli) -> Result<SequencerGateway> {
    if let Some(url) = &cli.base_url {
        let url = Url::parse(url).context("invalid base URL")?;
        return Ok(SequencerGateway::new(url));
    }

    let gateway = match cli.network {
        Network::Mainnet => SequencerGateway::sn_mainnet(),
        Network::Sepolia => SequencerGateway::sn_sepolia(),
    };

    Ok(gateway)
}

impl BlockIdArgs {
    fn resolve(&self) -> Result<BlockId> {
        if let Some(hash) = &self.block_hash {
            return Ok(BlockId::Hash(hash));
        }

        if let Some(number) = self.block_number {
            return Ok(BlockId::Number(number));
        }

        if self.latest {
            return Ok(BlockId::Latest);
        }

        Ok(BlockId::Latest)
    }
}

fn map_gateway_error(err: GatewayError) -> anyhow::Error {
    match err {
        GatewayError::RateLimited => anyhow!("request rate limited (HTTP 429)"),
        other => anyhow!(other),
    }
}

fn print_state_update(update: &StateUpdate) {
    match serde_json::to_string_pretty(update) {
        Ok(json) => println!("{}", json),
        Err(_) => print_debug(update),
    }
}

fn print_state_update_with_block(update: &StateUpdateWithBlock) {
    print_debug(update);
}

fn print_debug<T>(value: T)
where
    T: fmt::Debug,
{
    println!("{:#?}", value);
}
