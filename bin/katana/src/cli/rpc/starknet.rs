use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use katana_primitives::block::{BlockIdOrTag, BlockNumber, ConfirmedBlockIdOrTag};
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;

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
    /// Class hash
    class_hash: String,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,
}

#[derive(Debug, Args)]
pub struct GetClassHashAtArgs {
    /// Contract address
    contract_address: String,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,
}

#[derive(Debug, Args)]
pub struct GetClassAtArgs {
    /// Contract address
    contract_address: String,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,
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
    /// The contract address whose nonce is requested
    address: String,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,
}

#[derive(Debug, Args)]
pub struct GetStorageProofArgs {
    /// Class hashes JSON array
    #[arg(long)]
    class_hashes: Option<String>,

    /// Contract addresses JSON array
    #[arg(long)]
    contract_addresses: Option<String>,

    /// Contract storage keys JSON
    #[arg(long)]
    contracts_storage_keys: Option<String>,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block_id: BlockIdArg,
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
                let block_id = args.block_id.to_json();

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
                let block_id = args.block_id.to_json();
                let result = client.get_state_update(block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetStorageAt(args) => {
                let contract_address = Felt::from_str(&args.contract_address)?;
                let key = Felt::from_str(&args.key)?;
                let block_id = args.block_id.to_json();
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
                let block_id = args.block_id.to_json();
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
                let block_id = args.block_id.to_json();
                let class_hash = Felt::from_str(&args.class_hash).context("Invalid class hash")?;
                let result = client.get_class(block_id, class_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetClassHashAt(args) => {
                let block_id = args.block_id.to_json();
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_hash_at(block_id, contract_address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetClassAt(args) => {
                let block_id = args.block_id.to_json();
                let contract_address =
                    Felt::from_str(&args.contract_address).context("Invalid contract address")?;
                let result = client.get_class_at(block_id, contract_address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            StarknetCommands::GetBlockTransactionCount(args) => {
                let block_id = args.block_id.to_json();
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

                let request = serde_json::json!({
                    "contract_address": format!("{:#x}", contract_address),
                    "entry_point_selector": format!("{:#x}", entry_point_selector),
                    "calldata": calldata.iter().map(|f| format!("{:#x}", f)).collect::<Vec<_>>()
                });

                let block_id = args.block_id.to_json();
                let result = client.call(request, block_id).await?;
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
                let block_id = args.block_id.to_json();
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
                let result = client.trace_block_transactions(block_id.to_json()).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BlockIdArg(pub BlockIdOrTag);

impl BlockIdArg {
    /// Convert to JSON value for RPC request
    pub fn to_json(&self) -> serde_json::Value {
        match &self.0 {
            BlockIdOrTag::Latest => serde_json::json!("latest"),
            BlockIdOrTag::L1Accepted => serde_json::json!("l1_accepted"),
            BlockIdOrTag::PreConfirmed => serde_json::json!("pre_confirmed"),
            BlockIdOrTag::Hash(hash) => serde_json::json!({"block_hash": format!("{:#x}", hash)}),
            BlockIdOrTag::Number(num) => serde_json::json!({"block_number": num}),
        }
    }
}

impl std::str::FromStr for BlockIdArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let id = match s {
            "latest" => BlockIdOrTag::Latest,
            "l1_accepted" => BlockIdOrTag::L1Accepted,
            "pre_confirmed" | "preconfirmed" => BlockIdOrTag::PreConfirmed,

            hash if s.starts_with("0x") => BlockIdOrTag::Hash(
                Felt::from_hex(hash)
                    .with_context(|| format!("Invalid block hash format: {hash}"))?,
            ),

            num => BlockIdOrTag::Number(
                num.parse::<BlockNumber>()
                    .with_context(|| format!("Invalid block number format: {num}"))?,
            ),
        };

        Ok(BlockIdArg(id))
    }
}

impl Default for BlockIdArg {
    fn default() -> Self {
        BlockIdArg(BlockIdOrTag::Latest)
    }
}

#[derive(Debug, Clone)]
pub struct ConfirmedBlockIdArg(pub ConfirmedBlockIdOrTag);

impl ConfirmedBlockIdArg {
    /// Convert to JSON value for RPC request
    pub fn to_json(&self) -> serde_json::Value {
        match &self.0 {
            ConfirmedBlockIdOrTag::Latest => serde_json::json!("latest"),
            ConfirmedBlockIdOrTag::L1Accepted => serde_json::json!("l1_accepted"),
            ConfirmedBlockIdOrTag::Hash(hash) => {
                serde_json::json!({"block_hash": format!("{:#x}", hash)})
            }
            ConfirmedBlockIdOrTag::Number(num) => serde_json::json!({"block_number": num}),
        }
    }
}

impl std::str::FromStr for ConfirmedBlockIdArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let id = match s {
            "latest" => ConfirmedBlockIdOrTag::Latest,
            "l1_accepted" => ConfirmedBlockIdOrTag::L1Accepted,

            hash if s.starts_with("0x") => ConfirmedBlockIdOrTag::Hash(
                Felt::from_hex(hash)
                    .with_context(|| format!("Invalid block hash format: {hash}"))?,
            ),

            num => ConfirmedBlockIdOrTag::Number(
                num.parse::<BlockNumber>()
                    .with_context(|| format!("Invalid block number format: {num}"))?,
            ),
        };

        Ok(ConfirmedBlockIdArg(id))
    }
}

impl Default for ConfirmedBlockIdArg {
    fn default() -> Self {
        ConfirmedBlockIdArg(ConfirmedBlockIdOrTag::Latest)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use assert_matches::assert_matches;
    use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
    use katana_primitives::felt;

    use super::{BlockIdArg, ConfirmedBlockIdArg};

    #[test]
    fn block_id_arg_from_str() {
        // Test tag parsing
        let latest = BlockIdArg::from_str("latest").unwrap();
        assert_matches!(latest.0, BlockIdOrTag::Latest);

        let l1_accepted = BlockIdArg::from_str("l1_accepted").unwrap();
        assert_matches!(l1_accepted.0, BlockIdOrTag::L1Accepted);

        let preconfirmed = BlockIdArg::from_str("preconfirmed").unwrap();
        assert_matches!(preconfirmed.0, BlockIdOrTag::PreConfirmed);

        // Test hash parsing
        let hash = BlockIdArg::from_str("0x1234567890abcdef").unwrap();
        assert_matches!(hash.0, BlockIdOrTag::Hash(actual_hash) => {
            assert_eq!(actual_hash, felt!("0x1234567890abcdef"))
        });

        // Test number parsing
        let number = BlockIdArg::from_str("12345").unwrap();
        assert_matches!(number.0, BlockIdOrTag::Number(12345));

        // Test invalid hash
        assert!(BlockIdArg::from_str("0xinvalid").is_err());

        // Test invalid number
        assert!(BlockIdArg::from_str("not_a_number").is_err());
    }

    #[test]
    fn block_id_arg_default() {
        let default = BlockIdArg::default();
        assert_matches!(default.0, BlockIdOrTag::Latest);
    }

    #[test]
    fn confirmed_block_id_arg_from_str() {
        // Test tag parsing
        let latest = ConfirmedBlockIdArg::from_str("latest").unwrap();
        assert_matches!(latest.0, ConfirmedBlockIdOrTag::Latest);

        let l1_accepted = ConfirmedBlockIdArg::from_str("l1_accepted").unwrap();
        assert_matches!(l1_accepted.0, ConfirmedBlockIdOrTag::L1Accepted);

        // Test hash parsing
        let hash = ConfirmedBlockIdArg::from_str("0x1234567890abcdef").unwrap();
        assert_matches!(hash.0, ConfirmedBlockIdOrTag::Hash(actual_hash) => {
            assert_eq!(actual_hash, felt!("0x1234567890abcdef"))
        });

        // Test number parsing
        let number = ConfirmedBlockIdArg::from_str("12345").unwrap();
        assert_matches!(number.0, ConfirmedBlockIdOrTag::Number(12345));

        // Test invalid hash
        assert!(ConfirmedBlockIdArg::from_str("0xinvalid").is_err());

        // Test invalid number
        assert!(ConfirmedBlockIdArg::from_str("not_a_number").is_err());
    }

    #[test]
    fn confirmed_block_id_arg_default() {
        let default = ConfirmedBlockIdArg::default();
        assert_matches!(default.0, ConfirmedBlockIdOrTag::Latest);
    }
}
