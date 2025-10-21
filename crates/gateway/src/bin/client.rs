use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use katana_gateway::client::{Client, Error as GatewayError};
use katana_gateway::types::BlockId;
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::class::ClassHash;
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
    #[arg(global = true, long, value_enum, default_value_t = Network::Mainnet, conflicts_with_all = ["gateway_url", "feeder_gateway_url"])]
    network: Network,

    /// Override the gateway URL.
    #[arg(global = true, long, value_name = "URL", conflicts_with = "network")]
    #[arg(requires = "feeder_gateway_url")]
    gateway_url: Option<String>,

    /// Override the feeder gateway URL.
    #[arg(global = true, long, value_name = "URL", conflicts_with = "network")]
    #[arg(requires = "gateway_url")]
    feeder_gateway_url: Option<String>,

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
    /// Fetch a preconfirmed block.
    GetPreconfirmedBlock(PreConfirmedBlockCommand),
    /// Fetch state update information for a block.
    GetStateUpdate(StateUpdateCommand),
    /// Fetch a class definition by hash.
    GetClass(ClassCommand),
    /// Fetch a compiled class by hash.
    GetCompiledClass(ClassCommand),
    /// Fetch the public key for a sequencer address.
    GetPublicKey,
    /// Fetch the signature of a block.
    GetSignature(GetSignatureCommand),
}

#[derive(Debug, Args)]
struct BlockCommand {
    /// Block ID (number, hash, 'latest'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
struct PreConfirmedBlockCommand {
    /// The pre-confirmed block number.
    block: BlockNumber,
}

#[derive(Debug, Args)]
struct StateUpdateCommand {
    /// Block ID (number, hash, 'latest'). Defaults to 'latest'
    #[arg(long, default_value = "latest")]
    block: BlockIdArg,

    /// Include the full block alongside the state update.
    #[arg(long)]
    with_block: bool,
}

#[derive(Debug, Args)]
struct ClassCommand {
    /// Class hash to fetch.
    #[arg(value_name = "HASH")]
    class_hash: ClassHash,

    /// Block ID (number, hash, 'latest'). Defaults to 'latest'
    #[arg(long, default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
struct GetSignatureCommand {
    /// Block ID (number, hash, 'latest'). Defaults to 'latest'
    #[arg(long, default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Clone)]
struct BlockIdArg(BlockId);

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
            let block_id = cmd.block.0;
            let block = gateway.get_block(block_id).await.map_err(map_gateway_error)?;
            print_json(block)?;
        }
        Command::GetPreconfirmedBlock(cmd) => {
            let block_id = cmd.block;
            let block =
                gateway.get_preconfirmed_block(block_id).await.map_err(map_gateway_error)?;
            print_json(block)?;
        }
        Command::GetStateUpdate(cmd) => {
            let block_id = cmd.block.0;
            if cmd.with_block {
                let update = gateway
                    .get_state_update_with_block(block_id)
                    .await
                    .map_err(map_gateway_error)?;
                print_json(&update)?;
            } else {
                let update = gateway.get_state_update(block_id).await.map_err(map_gateway_error)?;
                print_json(&update)?;
            }
        }
        Command::GetClass(cmd) => {
            let block_id = cmd.block.0;
            let class_hash = cmd.class_hash;
            let class = gateway.get_class(class_hash, block_id).await.map_err(map_gateway_error)?;
            print_json(class)?;
        }
        Command::GetCompiledClass(cmd) => {
            let block_id = cmd.block.0;
            let class_hash = cmd.class_hash;
            let class = gateway
                .get_compiled_class(class_hash, block_id)
                .await
                .map_err(map_gateway_error)?;
            print_json(class)?;
        }
        Command::GetPublicKey => {
            let public_key = gateway.get_public_key().await.map_err(map_gateway_error)?;
            println!("{public_key:#x}")
        }
        Command::GetSignature(cmd) => {
            let block_id = cmd.block.0;
            let signature = gateway.get_signature(block_id).await.map_err(map_gateway_error)?;
            print_json(signature)?;
        }
    }

    Ok(())
}

fn build_gateway(cli: &Cli) -> Result<Client> {
    if let (Some(gateway_url), Some(feeder_url)) = (&cli.gateway_url, &cli.feeder_gateway_url) {
        let gateway = Url::parse(gateway_url).context("invalid gateway URL")?;
        let feeder = Url::parse(feeder_url).context("invalid feeder gateway URL")?;
        return Ok(Client::new(gateway, feeder));
    }

    if cli.gateway_url.is_some() || cli.feeder_gateway_url.is_some() {
        return Err(anyhow!(
            "both --gateway-url and --feeder-gateway-url must be provided when overriding URLs"
        ));
    }

    let gateway = match cli.network {
        Network::Mainnet => Client::mainnet(),
        Network::Sepolia => Client::sepolia(),
    };

    Ok(gateway)
}

impl FromStr for BlockIdArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let id = match s {
            "latest" => BlockId::Latest,

            "pending" => BlockId::Pending,

            hash if s.starts_with("0x") => BlockHash::from_hex(hash)
                .map(BlockId::Hash)
                .with_context(|| format!("Invalid block hash: {hash}"))?,

            num => num
                .parse::<BlockNumber>()
                .map(BlockId::Number)
                .with_context(|| format!("Invalid block number format: {num}"))?,
        };

        Ok(BlockIdArg(id))
    }
}

impl Default for BlockIdArg {
    fn default() -> Self {
        BlockIdArg(BlockId::Latest)
    }
}

fn map_gateway_error(err: GatewayError) -> anyhow::Error {
    match err {
        GatewayError::RateLimited => anyhow!("request rate limited (HTTP 429)"),
        other => anyhow!(other),
    }
}

fn print_json<T: serde::Serialize>(value: T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
