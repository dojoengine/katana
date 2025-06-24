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
    /// Katana RPC endpoint URL
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
    /// Read API methods
    #[command(subcommand)]
    Read(ReadCommand),

    /// Trace API methods
    #[command(subcommand)]
    Trace(TraceCommand),
}

// Read API Commands
#[derive(Debug, Subcommand)]
enum ReadCommand {
    /// Get Starknet JSON-RPC specification version
    SpecVersion,

    /// Get block with transaction hashes
    GetBlockWithTxHashes(BlockIdArgs),

    /// Get block with full transactions
    GetBlockWithTxs(BlockIdArgs),

    /// Get block with full transactions and receipts
    GetBlockWithReceipts(BlockIdArgs),

    /// Get state update for a block
    GetStateUpdate(BlockIdArgs),

    /// Get storage value at address and key
    GetStorageAt(GetStorageAtArgs),

    /// Get transaction status
    GetTransactionStatus(TxHashArgs),

    /// Get transaction by hash
    GetTransactionByHash(TxHashArgs),

    /// Get transaction by block ID and index
    GetTransactionByBlockIdAndIndex(GetTransactionByBlockIdAndIndexArgs),

    /// Get transaction receipt
    GetTransactionReceipt(TxHashArgs),

    /// Get contract class definition
    GetClass(GetClassArgs),

    /// Get contract class hash at address
    GetClassHashAt(GetClassHashAtArgs),

    /// Get contract class at address
    GetClassAt(GetClassAtArgs),

    /// Get number of transactions in block
    GetBlockTransactionCount(BlockIdArgs),

    /// Call contract function
    Call(CallArgs),

    /// Get latest block number
    BlockNumber,

    /// Get latest block hash and number
    BlockHashAndNumber,

    /// Get chain ID
    ChainId,

    /// Get sync status
    Syncing,

    /// Get nonce for address
    GetNonce(GetNonceArgs),
}

// Trace API Commands
#[derive(Debug, Subcommand)]
enum TraceCommand {
    /// Get transaction execution trace
    Transaction(TxHashArgs),

    /// Get execution traces for all transactions in a block
    BlockTransactions(BlockIdArgs),
}

#[derive(Debug, Args)]
struct BlockIdArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,
}

#[derive(Debug, Args)]
struct TxHashArgs {
    /// Transaction hash
    tx_hash: String,
}

#[derive(Debug, Args)]
struct GetStorageAtArgs {
    /// Contract address
    contract_address: String,

    /// Storage key
    key: String,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,
}

#[derive(Debug, Args)]
struct GetTransactionByBlockIdAndIndexArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Transaction index
    index: u64,
}

#[derive(Debug, Args)]
struct GetClassArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Class hash
    class_hash: String,
}

#[derive(Debug, Args)]
struct GetClassHashAtArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Contract address
    contract_address: String,
}

#[derive(Debug, Args)]
struct GetClassAtArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Contract address
    contract_address: String,
}

#[derive(Debug, Args)]
struct CallArgs {
    /// Contract address
    contract_address: String,

    /// Function selector
    selector: String,

    /// Calldata (space-separated hex values)
    calldata: Vec<String>,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,
}

#[derive(Debug, Args)]
struct GetEventsArgs {
    /// Event filter JSON
    filter: String,
}

#[derive(Debug, Args)]
struct GetNonceArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// The contract address whose nonce is requested
    address: String,
}

#[derive(Debug, Args)]
struct GetStorageProofArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Class hashes JSON array
    #[arg(long)]
    class_hashes: Option<String>,

    /// Contract addresses JSON array
    #[arg(long)]
    contract_addresses: Option<String>,

    /// Contract storage keys JSON
    #[arg(long)]
    contracts_storage_keys: Option<String>,
}

#[derive(Debug, Args)]
struct AddInvokeTransactionArgs {
    /// Invoke transaction JSON
    transaction: String,
}

#[derive(Debug, Args)]
struct AddDeclareTransactionArgs {
    /// Declare transaction JSON
    transaction: String,
}

#[derive(Debug, Args)]
struct AddDeployAccountTransactionArgs {
    /// Deploy account transaction JSON
    transaction: String,
}

#[derive(Debug, Args)]
struct SimulateTransactionsArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Transactions JSON (as array of broadcasted transactions)
    transactions: String,

    /// Simulation flags JSON array
    #[arg(long)]
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
