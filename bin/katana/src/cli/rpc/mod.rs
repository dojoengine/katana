use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_rpc_types::FunctionCall;
use starknet::core::types::{BlockId, BlockTag};

mod client;

use client::Client;
use url::Url;

#[derive(Debug, Args)]
pub struct RpcArgs {
    /// katana RPC endpoint URL
    #[arg(global = true)]
    #[arg(long, default_value = "http://localhost:5050")]
    url: String,

    #[command(subcommand)]
    command: RpcCommand,
}

impl RpcArgs {
    pub async fn execute(self) -> Result<()> {
        let client = self.client().context("Failed to create client")?;
        match self.command {
            RpcCommand::Read(args) => args.execute(&client).await,
            RpcCommand::Trace(args) => args.execute(&client).await,
        }
    }

    fn client(&self) -> Result<Client> {
        Ok(Client::new(Url::parse(&self.url)?))
    }
}

#[derive(Debug, Subcommand)]
enum RpcCommand {
    #[command(subcommand, about = "Read API methods")]
    Read(ReadCommand),

    #[command(subcommand, about = "Trace API methods")]
    Trace(TraceCommand),
}

// Read API Commands
#[derive(Debug, Subcommand)]
enum ReadCommand {
    #[command(about = "Get Starknet JSON-RPC specification version")]
    SpecVersion,

    #[command(about = "Get block with transaction hashes")]
    GetBlockWithTxHashes(BlockIdArgs),

    #[command(about = "Get block with full transactions")]
    GetBlockWithTxs(BlockIdArgs),

    #[command(about = "Get block with full transactions and receipts")]
    GetBlockWithReceipts(BlockIdArgs),

    #[command(about = "Get state update for a block")]
    GetStateUpdate(BlockIdArgs),

    #[command(about = "Get storage value at address and key")]
    GetStorageAt(GetStorageAtArgs),

    #[command(about = "Get transaction status")]
    GetTransactionStatus(TxHashArgs),

    #[command(about = "Get transaction by hash")]
    GetTransactionByHash(TxHashArgs),

    #[command(about = "Get transaction by block ID and index")]
    GetTransactionByBlockIdAndIndex(GetTransactionByBlockIdAndIndexArgs),

    #[command(about = "Get transaction receipt")]
    GetTransactionReceipt(TxHashArgs),

    #[command(about = "Get contract class definition")]
    GetClass(GetClassArgs),

    #[command(about = "Get contract class hash at address")]
    GetClassHashAt(GetClassHashAtArgs),

    #[command(about = "Get contract class at address")]
    GetClassAt(GetClassAtArgs),

    #[command(about = "Get number of transactions in block")]
    GetBlockTransactionCount(BlockIdArgs),

    #[command(about = "Call contract function")]
    Call(CallArgs),

    #[command(about = "Get latest block number")]
    BlockNumber,

    #[command(about = "Get latest block hash and number")]
    BlockHashAndNumber,

    #[command(about = "Get chain ID")]
    ChainId,

    #[command(about = "Get sync status")]
    Syncing,

    #[command(about = "Get nonce for address")]
    GetNonce(GetNonceArgs),
}

// Trace API Commands
#[derive(Debug, Subcommand)]
enum TraceCommand {
    #[command(about = "Get transaction execution trace")]
    Transaction(TxHashArgs),

    #[command(about = "Get execution traces for all transactions in a block")]
    BlockTransactions(BlockIdArgs),
}

#[derive(Debug, Args)]
struct BlockIdArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
}

#[derive(Debug, Args)]
struct TxHashArgs {
    #[arg(help = "Transaction hash")]
    tx_hash: String,
}

#[derive(Debug, Args)]
struct GetStorageAtArgs {
    #[arg(help = "Contract address")]
    contract_address: String,

    #[arg(help = "Storage key")]
    key: String,

    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
}

#[derive(Debug, Args)]
struct GetTransactionByBlockIdAndIndexArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,

    #[arg(help = "Transaction index")]
    index: u64,
}

#[derive(Debug, Args)]
struct GetClassArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,

    #[arg(help = "Class hash")]
    class_hash: String,
}

#[derive(Debug, Args)]
struct GetClassHashAtArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,

    #[arg(help = "Contract address")]
    contract_address: String,
}

#[derive(Debug, Args)]
struct GetClassAtArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,

    #[arg(help = "Contract address")]
    contract_address: String,
}

#[derive(Debug, Args)]
struct CallArgs {
    #[arg(help = "Contract address")]
    contract_address: String,

    #[arg(help = "Function selector")]
    selector: String,

    #[arg(help = "Calldata (space-separated hex values)")]
    calldata: Vec<String>,

    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
}

#[derive(Debug, Args)]
struct GetEventsArgs {
    #[arg(help = "Event filter JSON")]
    filter: String,
}

#[derive(Debug, Args)]
struct GetNonceArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,

    /// The contract address whose nonce is requested
    address: String,
}

#[derive(Debug, Args)]
struct GetStorageProofArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,

    #[arg(long, help = "Class hashes JSON array")]
    class_hashes: Option<String>,

    #[arg(long, help = "Contract addresses JSON array")]
    contract_addresses: Option<String>,

    #[arg(long, help = "Contract storage keys JSON")]
    contracts_storage_keys: Option<String>,
}

#[derive(Debug, Args)]
struct AddInvokeTransactionArgs {
    #[arg(help = "Invoke transaction JSON")]
    transaction: String,
}

#[derive(Debug, Args)]
struct AddDeclareTransactionArgs {
    #[arg(help = "Declare transaction JSON")]
    transaction: String,
}

#[derive(Debug, Args)]
struct AddDeployAccountTransactionArgs {
    #[arg(help = "Deploy account transaction JSON")]
    transaction: String,
}

#[derive(Debug, Args)]
struct SimulateTransactionsArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,

    #[arg(help = "Transactions JSON (as array of broadcasted transactions)")]
    transactions: String,

    #[arg(long, help = "Simulation flags JSON array")]
    simulation_flags: Option<String>,
}

impl ReadCommand {
    async fn execute(self, client: &Client) -> Result<()> {
        match self {
            ReadCommand::SpecVersion => {
                let result = client.spec_version().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetBlockWithTxHashes(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_with_tx_hashes(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetBlockWithTxs(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_with_txs(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetBlockWithReceipts(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_with_receipts(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetStateUpdate(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_state_update(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetStorageAt(args) => {
                let contract_address = Felt::from_str(&args.contract_address)?;
                let key = Felt::from_str(&args.key)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_storage_at(contract_address, key, block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetTransactionStatus(args) => {
                let tx_hash =
                    TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_status(tx_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetTransactionByHash(args) => {
                let tx_hash =
                    TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_by_hash(tx_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetTransactionByBlockIdAndIndex(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result =
                    client.get_transaction_by_block_id_and_index(block_id, args.index).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetTransactionReceipt(args) => {
                let tx_hash =
                    TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_receipt(tx_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetClass(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let class_hash = Felt::from_str(&args.class_hash).context("Invalid class hash")?;
                let result = client.get_class(block_id, class_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetClassHashAt(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_hash_at(block_id, contract_address).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetClassAt(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_at(block_id, contract_address).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetBlockTransactionCount(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_transaction_count(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::Call(args) => {
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let entry_point_selector =
                    Felt::from_str(&args.selector).context("Invalid function selector")?;
                let calldata: Result<Vec<Felt>, _> = args
                    .calldata
                    .iter()
                    .map(|s| Felt::from_str(s).context("Invalid calldata value"))
                    .collect();
                let calldata = calldata?;

                let function_call =
                    FunctionCall { contract_address, entry_point_selector, calldata };

                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.call(function_call, block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::BlockNumber => {
                let result = client.block_number().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::BlockHashAndNumber => {
                let result = client.block_hash_and_number().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::ChainId => {
                let result = client.chain_id().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::Syncing => {
                let result = client.syncing().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetNonce(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let address = Felt::from_str(&args.address).context("Invalid contract address")?;
                let result = client.get_nonce(block_id, address).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }
        Ok(())
    }
}

// Implementation for Trace commands
impl TraceCommand {
    async fn execute(self, client: &Client) -> Result<()> {
        match self {
            TraceCommand::Transaction(args) => {
                let tx_hash =
                    TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.trace_transaction(tx_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            TraceCommand::BlockTransactions(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.trace_block_transactions(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }
        Ok(())
    }
}

fn parse_block_id(block_id: Option<&str>) -> Result<BlockId> {
    let Some(id) = block_id else { return Ok(BlockId::Tag(BlockTag::Latest)) };

    match id {
        "latest" => Ok(BlockId::Tag(BlockTag::Latest)),
        "pending" => Ok(BlockId::Tag(BlockTag::Pending)),

        hash if id.starts_with("0x") => {
            Ok(BlockId::Hash(Felt::from_hex(hash).context("Invalid block hash format")?))
        }

        num => {
            Ok(BlockId::Number(num.parse::<BlockNumber>().context("Invalid block number format")?))
        }
    }
}
