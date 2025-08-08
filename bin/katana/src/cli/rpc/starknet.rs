use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use starknet::core::types::{BlockId, BlockTag, ConfirmedBlockId, FunctionCall};

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
    GetTransactionByHash(GetTransactionArgs),

    /// Get transaction by block ID and index
    #[command(name = "tx-by-block")]
    GetTransactionByBlockIdAndIndex(GetTransactionByBlockIdAndIndexArgs),

    /// Get transaction receipt
    #[command(name = "receipt")]
    GetTransactionReceipt(TxHashArgs),

    /// Get contract class definition
    #[command(name = "class")]
    GetClass(GetClassArgs),

    /// Get contract class hash at address
    #[command(name = "class-at")]
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
    TraceBlockTransactions(TraceBlockTransactionsArg),
}

#[derive(Debug, Args)]
pub struct BlockIdArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,
}

#[derive(Debug, Args)]
pub struct GetBlockArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,

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
pub struct GetTransactionArgs {
    /// Transaction hash
    tx_hash: String,

    /// Get only the transaction status instead of full transaction
    #[arg(long)]
    status: bool,
}

#[derive(Debug, Args)]
pub struct GetStorageAtArgs {
    /// Contract address
    contract_address: String,

    /// Storage key
    key: String,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,
}

#[derive(Debug, Args)]
pub struct GetTransactionByBlockIdAndIndexArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,

    /// Transaction index
    index: u64,
}

#[derive(Debug, Args)]
pub struct GetClassArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,

    /// Class hash
    class_hash: String,
}

#[derive(Debug, Args)]
pub struct GetClassHashAtArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,

    /// Contract address
    contract_address: String,
}

#[derive(Debug, Args)]
pub struct GetClassAtArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,

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
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,
}

#[derive(Debug, Args)]
pub struct GetEventsArgs {
    /// Event filter JSON
    filter: String,
}

#[derive(Debug, Args)]
pub struct GetNonceArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,

    /// The contract address whose nonce is requested
    address: String,
}

#[derive(Debug, Args)]
pub struct GetStorageProofArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,

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
    /// Block ID (number, hash, 'latest', or 'preconfirmed'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,

    /// Transactions JSON (as array of broadcasted transactions)
    transactions: String,

    /// Simulation flags JSON array
    #[arg(long)]
    simulation_flags: Option<String>,
}

#[derive(Debug, Args)]
pub struct TraceBlockTransactionsArg {
    /// Block ID (number, hash, 'latest', or 'l1_accepted'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: ConfirmedBlockIdArg,
}

impl StarknetCommands {
    pub async fn execute(self, client: &Client) -> Result<()> {
        match self {
            StarknetCommands::SpecVersion => {
                let result = client.spec_version().await?;
                println!("{result}");
            }
            StarknetCommands::GetBlockWithTxs(args) => {
                let block_id = args.block_id.0;

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
                let block_id = args.block_id.0;
                let result = client.get_state_update(block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetStorageAt(args) => {
                let contract_address = Felt::from_str(&args.contract_address)?;
                let key = Felt::from_str(&args.key)?;
                let block_id = args.block_id.0;
                let result = client.get_storage_at(contract_address, key, block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetTransactionByHash(args) => {
                let hash = TxHash::from_str(&args.tx_hash).context("Invalid transaction hash")?;

                if args.status {
                    let result = client.get_transaction_status(hash).await?;
                    println!("{}", colored_json::to_colored_json_auto(&result)?);
                } else {
                    let result = client.get_transaction_by_hash(hash).await?;
                    println!("{}", colored_json::to_colored_json_auto(&result)?);
                };
            }
            StarknetCommands::GetTransactionByBlockIdAndIndex(args) => {
                let block_id = args.block_id.0;
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
                let block_id = args.block_id.0;
                let class_hash = Felt::from_str(&args.class_hash).context("Invalid class hash")?;
                let result = client.get_class(block_id, class_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetClassHashAt(args) => {
                let block_id = args.block_id.0;
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_hash_at(block_id, contract_address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetClassAt(args) => {
                let block_id = args.block_id.0;
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_at(block_id, contract_address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetBlockTransactionCount(args) => {
                let block_id = args.block_id.0;
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

                let block_id = args.block_id.0;
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
                let block_id = args.block_id.0;
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
            StarknetCommands::TraceBlockTransactions(TraceBlockTransactionsArg { block_id }) => {
                let result = client.trace_block_transactions(block_id.0).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BlockIdArg(pub BlockId);

impl std::str::FromStr for BlockIdArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let id = match s {
            "latest" => BlockId::Tag(BlockTag::Latest),
            "l1_accepted" => BlockId::Tag(BlockTag::L1Accepted),
            "preconfirmed" => BlockId::Tag(BlockTag::PreConfirmed),

            hash if s.starts_with("0x") => BlockId::Hash(
                Felt::from_hex(hash)
                    .with_context(|| format!("Invalid block hash format: {hash}"))?,
            ),

            num => BlockId::Number(
                num.parse::<BlockNumber>()
                    .with_context(|| format!("Invalid block number format: {num}"))?,
            ),
        };

        Ok(BlockIdArg(id))
    }
}

impl Default for BlockIdArg {
    fn default() -> Self {
        BlockIdArg(BlockId::Tag(BlockTag::Latest))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConfirmedBlockIdArg(pub ConfirmedBlockId);

impl std::str::FromStr for ConfirmedBlockIdArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let id = match s {
            "latest" => ConfirmedBlockId::Latest,
            "l1_accepted" => ConfirmedBlockId::L1Accepted,

            hash if s.starts_with("0x") => ConfirmedBlockId::Hash(
                Felt::from_hex(hash)
                    .with_context(|| format!("Invalid block hash format: {hash}"))?,
            ),

            num => ConfirmedBlockId::Number(
                num.parse::<BlockNumber>()
                    .with_context(|| format!("Invalid block number format: {num}"))?,
            ),
        };

        Ok(ConfirmedBlockIdArg(id))
    }
}

impl Default for ConfirmedBlockIdArg {
    fn default() -> Self {
        ConfirmedBlockIdArg(ConfirmedBlockId::Latest)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use starknet::core::types::{BlockId, BlockTag, ConfirmedBlockId};

    use super::{BlockIdArg, ConfirmedBlockIdArg};

    #[test]
    fn block_id_arg_from_str() {
        // Test tag parsing
        let latest = BlockIdArg::from_str("latest").unwrap();
        assert!(matches!(latest.0, BlockId::Tag(BlockTag::Latest)));

        let l1_accepted = BlockIdArg::from_str("l1_accepted").unwrap();
        assert!(matches!(l1_accepted.0, BlockId::Tag(BlockTag::L1Accepted)));

        let preconfirmed = BlockIdArg::from_str("preconfirmed").unwrap();
        assert!(matches!(preconfirmed.0, BlockId::Tag(BlockTag::PreConfirmed)));

        // Test hash parsing
        let hash = BlockIdArg::from_str("0x1234567890abcdef").unwrap();
        assert!(matches!(hash.0, BlockId::Hash(_)));

        // Test number parsing
        let number = BlockIdArg::from_str("12345").unwrap();
        assert!(matches!(number.0, BlockId::Number(12345)));

        // Test invalid hash
        assert!(BlockIdArg::from_str("0xinvalid").is_err());

        // Test invalid number
        assert!(BlockIdArg::from_str("not_a_number").is_err());
    }

    #[test]
    fn block_id_arg_default() {
        let default = BlockIdArg::default();
        assert!(matches!(default.0, BlockId::Tag(BlockTag::Latest)));
    }

    #[test]
    fn confirmed_block_id_arg_from_str() {
        // Test tag parsing
        let latest = ConfirmedBlockIdArg::from_str("latest").unwrap();
        assert!(matches!(latest.0, ConfirmedBlockId::Latest));

        let l1_accepted = ConfirmedBlockIdArg::from_str("l1_accepted").unwrap();
        assert!(matches!(l1_accepted.0, ConfirmedBlockId::L1Accepted));

        // Test hash parsing
        let hash = ConfirmedBlockIdArg::from_str("0x1234567890abcdef").unwrap();
        assert!(matches!(hash.0, ConfirmedBlockId::Hash(_)));

        // Test number parsing
        let number = ConfirmedBlockIdArg::from_str("12345").unwrap();
        assert!(matches!(number.0, ConfirmedBlockId::Number(12345)));

        // Test invalid hash
        assert!(ConfirmedBlockIdArg::from_str("0xinvalid").is_err());

        // Test invalid number
        assert!(ConfirmedBlockIdArg::from_str("not_a_number").is_err());
    }

    #[test]
    fn confirmed_block_id_arg_default() {
        let default = ConfirmedBlockIdArg::default();
        assert!(matches!(default.0, ConfirmedBlockId::Latest));
    }
}
