use std::str::FromStr;

use anyhow::{Context, Result};  
use clap::{Args, Subcommand};
use katana_primitives::block::{BlockIdOrTag, BlockNumber, BlockTag};
use katana_primitives::class::ClassHash;
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::event::EventFilterWithPage;
use katana_rpc_types::message::MsgFromL1;
use katana_rpc_types::transaction::{BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx, BroadcastedTx};
use katana_rpc_types::trie::ContractStorageKeys;
use katana_rpc_types::{FunctionCall, SimulationFlag, SimulationFlagForEstimateFee};
use serde_json::Value;
use starknet::core::types::BlockId;

mod client;

use client::RpcClient;

#[derive(Debug, Args)]
pub struct RpcArgs {
    #[command(subcommand)]
    command: RpcCommand,
}

impl RpcArgs {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            RpcCommand::Read(args) => args.execute().await,
            RpcCommand::Write(args) => args.execute().await,
            RpcCommand::Trace(args) => args.execute().await,
        }
    }
}

#[derive(Debug, Subcommand)]
enum RpcCommand {
    #[command(subcommand, about = "Read API methods")]
    Read(ReadCommand),
    
    #[command(subcommand, about = "Write API methods")]
    Write(WriteCommand),
    
    #[command(subcommand, about = "Trace API methods")]
    Trace(TraceCommand),
}

// Read API Commands
#[derive(Debug, Subcommand)]
enum ReadCommand {
    #[command(about = "Get Starknet JSON-RPC specification version")]
    SpecVersion(BaseArgs),
    
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
    
    #[command(about = "Estimate transaction fees")]
    EstimateFee(EstimateFeeArgs),
    
    #[command(about = "Estimate L2 fee for L1 message")]
    EstimateMessageFee(EstimateMessageFeeArgs),
    
    #[command(about = "Get latest block number")]
    BlockNumber(BaseArgs),
    
    #[command(about = "Get latest block hash and number")]
    BlockHashAndNumber(BaseArgs),
    
    #[command(about = "Get chain ID")]
    ChainId(BaseArgs),
    
    #[command(about = "Get sync status")]
    Syncing(BaseArgs),
    
    #[command(about = "Get events matching filter")]
    GetEvents(GetEventsArgs),
    
    #[command(about = "Get nonce for address")]
    GetNonce(GetNonceArgs),
    
    #[command(about = "Get storage proof")]
    GetStorageProof(GetStorageProofArgs),
}

// Write API Commands
#[derive(Debug, Subcommand)]
enum WriteCommand {
    #[command(about = "Submit invoke transaction")]
    AddInvokeTransaction(AddInvokeTransactionArgs),
    
    #[command(about = "Submit declare transaction")]
    AddDeclareTransaction(AddDeclareTransactionArgs),
    
    #[command(about = "Submit deploy account transaction")]
    AddDeployAccountTransaction(AddDeployAccountTransactionArgs),
}

// Trace API Commands
#[derive(Debug, Subcommand)]
enum TraceCommand {
    #[command(about = "Get transaction execution trace")]
    TraceTransaction(TxHashArgs),
    
    #[command(about = "Simulate transactions")]
    SimulateTransactions(SimulateTransactionsArgs),
    
    #[command(about = "Get execution traces for all transactions in a block")]
    TraceBlockTransactions(BlockIdArgs),
}

// Command argument structures
#[derive(Debug, Args)]
struct BaseArgs {
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct BlockIdArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct TxHashArgs {
    #[arg(help = "Transaction hash")]
    tx_hash: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct GetStorageAtArgs {
    #[arg(help = "Contract address")]
    contract_address: String,
    
    #[arg(help = "Storage key")]
    key: String,
    
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct GetTransactionByBlockIdAndIndexArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(help = "Transaction index")]
    index: u64,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct GetClassArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(help = "Class hash")]
    class_hash: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct GetClassHashAtArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(help = "Contract address")]
    contract_address: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct GetClassAtArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(help = "Contract address")]
    contract_address: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
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
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct EstimateFeeArgs {
    #[arg(help = "Transactions JSON (as array of broadcasted transactions)")]
    transactions: String,
    
    #[arg(long, help = "Simulation flags JSON array")]
    simulation_flags: Option<String>,
    
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct EstimateMessageFeeArgs {
    #[arg(help = "L1 message JSON")]
    message: String,
    
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct GetEventsArgs {
    #[arg(help = "Event filter JSON")]
    filter: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct GetNonceArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(help = "Contract address")]
    contract_address: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
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
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct AddInvokeTransactionArgs {
    #[arg(help = "Invoke transaction JSON")]
    transaction: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct AddDeclareTransactionArgs {
    #[arg(help = "Declare transaction JSON")]
    transaction: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct AddDeployAccountTransactionArgs {
    #[arg(help = "Deploy account transaction JSON")]
    transaction: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

#[derive(Debug, Args)]
struct SimulateTransactionsArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(help = "Transactions JSON (as array of broadcasted transactions)")]
    transactions: String,
    
    #[arg(long, help = "Simulation flags JSON array")]
    simulation_flags: Option<String>,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

// Implementation for Read commands
impl ReadCommand {
    async fn execute(self) -> Result<()> {
        match self {
            ReadCommand::SpecVersion(args) => {
                let client = RpcClient::new(&args.url)?;
                let result = client.spec_version().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetBlockWithTxHashes(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_with_tx_hashes(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetBlockWithTxs(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_with_txs(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetBlockWithReceipts(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_with_receipts(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetStateUpdate(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_state_update(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetStorageAt(args) => {
                let client = RpcClient::new(&args.url)?;
                let contract_address = Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let key = Felt::from_str(&args.key).context("Invalid storage key")?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_storage_at(contract_address, key, block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetTransactionStatus(args) => {
                let client = RpcClient::new(&args.url)?;
                let tx_hash = TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_status(tx_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetTransactionByHash(args) => {
                let client = RpcClient::new(&args.url)?;
                let tx_hash = TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_by_hash(tx_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetTransactionByBlockIdAndIndex(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_transaction_by_block_id_and_index(block_id, args.index).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetTransactionReceipt(args) => {
                let client = RpcClient::new(&args.url)?;
                let tx_hash = TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.get_transaction_receipt(tx_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetClass(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let class_hash = Felt::from_str(&args.class_hash).context("Invalid class hash")?;
                let result = client.get_class(block_id, class_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetClassHashAt(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let contract_address = Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_hash_at(block_id, contract_address).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetClassAt(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let contract_address = Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_at(block_id, contract_address).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetBlockTransactionCount(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.get_block_transaction_count(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::Call(args) => {
                let client = RpcClient::new(&args.url)?;
                let contract_address = Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let entry_point_selector = Felt::from_str(&args.selector).context("Invalid function selector")?;
                let calldata: Result<Vec<Felt>, _> = args.calldata.iter()
                    .map(|s| Felt::from_str(s).context("Invalid calldata value"))
                    .collect();
                let calldata = calldata?;
                
                let function_call = FunctionCall {
                    contract_address,
                    entry_point_selector,
                    calldata,
                };
                
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.call(function_call, block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::EstimateFee(args) => {
                let client = RpcClient::new(&args.url)?;
                let transactions: Vec<BroadcastedTx> = serde_json::from_str(&args.transactions)
                    .context("Failed to parse transactions JSON")?;
                let simulation_flags: Vec<SimulationFlagForEstimateFee> = if let Some(flags) = args.simulation_flags {
                    serde_json::from_str(&flags).context("Failed to parse simulation flags JSON")?
                } else {
                    vec![]
                };
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.estimate_fee(transactions, simulation_flags, block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::EstimateMessageFee(args) => {
                let client = RpcClient::new(&args.url)?;
                let message: MsgFromL1 = serde_json::from_str(&args.message)
                    .context("Failed to parse message JSON")?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.estimate_message_fee(message, block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::BlockNumber(args) => {
                let client = RpcClient::new(&args.url)?;
                let result = client.block_number().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::BlockHashAndNumber(args) => {
                let client = RpcClient::new(&args.url)?;
                let result = client.block_hash_and_number().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::ChainId(args) => {
                let client = RpcClient::new(&args.url)?;
                let result = client.chain_id().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::Syncing(args) => {
                let client = RpcClient::new(&args.url)?;
                let result = client.syncing().await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetEvents(args) => {
                let client = RpcClient::new(&args.url)?;
                let filter: EventFilterWithPage = serde_json::from_str(&args.filter)
                    .context("Failed to parse event filter JSON")?;
                let result = client.get_events(filter).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetNonce(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let contract_address = Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_nonce(block_id, contract_address).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            ReadCommand::GetStorageProof(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let class_hashes: Option<Vec<ClassHash>> = if let Some(hashes) = args.class_hashes {
                    Some(serde_json::from_str(&hashes).context("Failed to parse class hashes JSON")?)
                } else {
                    None
                };
                let contract_addresses: Option<Vec<ContractAddress>> = if let Some(addresses) = args.contract_addresses {
                    Some(serde_json::from_str(&addresses).context("Failed to parse contract addresses JSON")?)
                } else {
                    None
                };
                let contracts_storage_keys: Option<Vec<ContractStorageKeys>> = if let Some(keys) = args.contracts_storage_keys {
                    Some(serde_json::from_str(&keys).context("Failed to parse contract storage keys JSON")?)
                } else {
                    None
                };
                let result = client.get_storage_proof(block_id, class_hashes, contract_addresses, contracts_storage_keys).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }
        Ok(())
    }
}

// Implementation for Write commands
impl WriteCommand {
    async fn execute(self) -> Result<()> {
        match self {
            WriteCommand::AddInvokeTransaction(args) => {
                let client = RpcClient::new(&args.url)?;
                let transaction: BroadcastedInvokeTx = serde_json::from_str(&args.transaction)
                    .context("Failed to parse invoke transaction JSON")?;
                let result = client.add_invoke_transaction(transaction).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            WriteCommand::AddDeclareTransaction(args) => {
                let client = RpcClient::new(&args.url)?;
                let transaction: BroadcastedDeclareTx = serde_json::from_str(&args.transaction)
                    .context("Failed to parse declare transaction JSON")?;
                let result = client.add_declare_transaction(transaction).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            WriteCommand::AddDeployAccountTransaction(args) => {
                let client = RpcClient::new(&args.url)?;
                let transaction: BroadcastedDeployAccountTx = serde_json::from_str(&args.transaction)
                    .context("Failed to parse deploy account transaction JSON")?;
                let result = client.add_deploy_account_transaction(transaction).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }
        Ok(())
    }
}

// Implementation for Trace commands
impl TraceCommand {
    async fn execute(self) -> Result<()> {
        match self {
            TraceCommand::TraceTransaction(args) => {
                let client = RpcClient::new(&args.url)?;
                let tx_hash = TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;
                let result = client.trace_transaction(tx_hash).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            TraceCommand::SimulateTransactions(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let transactions: Vec<BroadcastedTx> = serde_json::from_str(&args.transactions)
                    .context("Failed to parse transactions JSON")?;
                let simulation_flags: Vec<SimulationFlag> = if let Some(flags) = args.simulation_flags {
                    serde_json::from_str(&flags).context("Failed to parse simulation flags JSON")?
                } else {
                    vec![]
                };
                let result = client.simulate_transactions(block_id, transactions, simulation_flags).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            TraceCommand::TraceBlockTransactions(args) => {
                let client = RpcClient::new(&args.url)?;
                let block_id = parse_block_id(args.block_id.as_deref())?;
                let result = client.trace_block_transactions(block_id).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }
        Ok(())
    }
}

fn parse_block_id(block_id: Option<&str>) -> Result<BlockIdOrTag> {
    let block_id = block_id.unwrap_or("latest");
    
    match block_id {
        "latest" => Ok(BlockId::Tag(BlockTag::Latest)),
        "pending" => Ok(BlockId::Tag(BlockTag::Pending)),
        _ => {
            if block_id.starts_with("0x") {
                let hash = Felt::from_str(block_id)
                    .context("Invalid block hash format")?;
                Ok(BlockId::Hash(hash))
            } else {
                let number = block_id.parse::<BlockNumber>()
                    .context("Invalid block number format")?;
                Ok(BlockId::Number(number))
            }
        }
    }
}