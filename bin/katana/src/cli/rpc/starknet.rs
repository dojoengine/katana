use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_rpc_types::FunctionCall;
use starknet::core::types::{BlockId, BlockTag};

use super::client::Client;

#[derive(Debug, Subcommand)]
pub enum StarknetCommands {
    /// Get Starknet JSON-RPC specification version
    #[command(name = "spec")]
    SpecVersion,

    /// Get block with full transactions
    #[command(name = "block")]
    GetBlockWithTxs(GetBlockArgs),

    /// Get state update for a block
    #[command(name = "state-update")]
    GetStateUpdate(BlockIdArgs),

    /// Get storage value at address and key
    #[command(name = "storage")]
    GetStorageAt(GetStorageAtArgs),

    /// Get transaction by hash
    #[command(name = "tx")]
    GetTransactionByHash(TxHashArgs),

    /// Get transaction status
    #[command(name = "tx-status")]
    GetTransactionStatus(TxHashArgs),

    /// Get transaction by block ID and index
    GetTransactionByBlockIdAndIndex(GetTransactionByBlockIdAndIndexArgs),

    /// Get transaction receipt
    #[command(name = "receipt")]
    GetTransactionReceipt(TxHashArgs),

    /// Get contract class definition
    #[command(name = "class")]
    GetClass(GetClassArgs),

    /// Get contract class hash at address
    GetClassHashAt(GetClassHashAtArgs),

    /// Get contract class at address
    #[command(name = "code")]
    GetClassAt(GetClassAtArgs),

    /// Get number of transactions in block
    #[command(name = "tx-count")]
    GetBlockTransactionCount(BlockIdArgs),

    /// Call contract function
    #[command(name = "call")]
    Call(CallArgs),

    /// Get latest block number
    #[command(name = "block-number")]
    BlockNumber,

    /// Get latest block hash and number
    BlockHashAndNumber,

    /// Get chain ID
    #[command(name = "id")]
    ChainId,

    /// Get sync status
    #[command(name = "sync")]
    Syncing,

    /// Get nonce for address
    #[command(name = "nonce")]
    GetNonce(GetNonceArgs),

    /// Get transaction execution trace
    #[command(name = "trace")]
    TraceTransaction(TxHashArgs),

    /// Get execution traces for all transactions in a block
    #[command(name = "block-traces")]
    TraceBlockTransactions(BlockIdArgs),
}

#[derive(Debug, Args)]
pub struct BlockIdArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct GetBlockArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Return block with receipts
    #[arg(long)]
    receipts: bool,

    /// Return only transaction hashes instead of full transactions
    #[arg(long, conflicts_with = "receipts")]
    tx_hashes_only: bool,
}

#[derive(Debug, Args)]
pub struct TxHashArgs {
    /// Transaction hash
    tx_hash: String,
}

#[derive(Debug, Args)]
pub struct GetStorageAtArgs {
    /// Contract address
    contract_address: String,

    /// Storage key
    key: String,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct GetTransactionByBlockIdAndIndexArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Transaction index
    index: u64,
}

#[derive(Debug, Args)]
pub struct GetClassArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Class hash
    class_hash: String,
}

#[derive(Debug, Args)]
pub struct GetClassHashAtArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Contract address
    contract_address: String,
}

#[derive(Debug, Args)]
pub struct GetClassAtArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Contract address
    contract_address: String,
}

#[derive(Debug, Args)]
pub struct CallArgs {
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
pub struct GetEventsArgs {
    /// Event filter JSON
    filter: String,
}

#[derive(Debug, Args)]
pub struct GetNonceArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// The contract address whose nonce is requested
    address: String,
}

#[derive(Debug, Args)]
pub struct GetStorageProofArgs {
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
pub struct AddInvokeTransactionArgs {
    /// Invoke transaction JSON
    transaction: String,
}

#[derive(Debug, Args)]
pub struct AddDeclareTransactionArgs {
    /// Declare transaction JSON
    transaction: String,
}

#[derive(Debug, Args)]
pub struct AddDeployAccountTransactionArgs {
    /// Deploy account transaction JSON
    transaction: String,
}

#[derive(Debug, Args)]
pub struct SimulateTransactionsArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    block_id: Option<String>,

    /// Transactions JSON (as array of broadcasted transactions)
    transactions: String,

    /// Simulation flags JSON array
    #[arg(long)]
    simulation_flags: Option<String>,
}

impl StarknetCommands {
    pub async fn execute(self, client: &Client) -> Result<()> {
        match self {
            StarknetCommands::SpecVersion => {
                let result = client.spec_version().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetBlockWithTxs(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;

                if args.receipts {
                    let result = client.get_block_with_receipts(block_id).await?;
                    println!("{}", colored_json::to_colored_json_auto(&result)?);
                } else if args.tx_hashes_only {
                    let result = client.get_block_with_tx_hashes(block_id).await?;
                    println!("{}", colored_json::to_colored_json_auto(&result)?);
                } else {
                    let result = client.get_block_with_txs(block_id).await?;
                    println!("{}", colored_json::to_colored_json_auto(&result)?);
                };
            }
            StarknetCommands::GetStateUpdate(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_state_update(block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetStorageAt(args) => {
                let contract_address = Felt::from_str(&args.contract_address)?;
                let key = Felt::from_str(&args.key)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_storage_at(contract_address, key, block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetTransactionStatus(args) => {
                let tx_hash =
                    TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_status(tx_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetTransactionByHash(args) => {
                let tx_hash =
                    TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_by_hash(tx_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetTransactionByBlockIdAndIndex(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result =
                    client.get_transaction_by_block_id_and_index(block_id, args.index).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetTransactionReceipt(args) => {
                let tx_hash =
                    TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_receipt(tx_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetClass(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let class_hash = Felt::from_str(&args.class_hash).context("Invalid class hash")?;
                let result = client.get_class(block_id, class_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetClassHashAt(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_hash_at(block_id, contract_address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetClassAt(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_at(block_id, contract_address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetBlockTransactionCount(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_transaction_count(block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::Call(args) => {
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
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::BlockNumber => {
                let result = client.block_number().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::BlockHashAndNumber => {
                let result = client.block_hash_and_number().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::ChainId => {
                let result = client.chain_id().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::Syncing => {
                let result = client.syncing().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetNonce(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let address = Felt::from_str(&args.address).context("Invalid contract address")?;
                let result = client.get_nonce(block_id, address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::TraceTransaction(args) => {
                let tx_hash =
                    TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.trace_transaction(tx_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::TraceBlockTransactions(args) => {
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.trace_block_transactions(block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
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
