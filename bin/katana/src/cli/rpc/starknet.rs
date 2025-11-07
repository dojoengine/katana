use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber, ConfirmedBlockIdOrTag};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::StorageKey;
use katana_primitives::execution::{EntryPointSelector, FunctionCall};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::event::{EventFilter, EventFilterWithPage, ResultPageRequest};

use super::client::Client;

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
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

    /// Get events matching filter criteria
    #[command(name = "events")]
    GetEvents(GetEventsArgs),

    /// Get transaction execution trace
    #[command(name = "trace")]
    TraceTransaction(TxHashArgs),

    /// Get execution traces for all transactions in a block
    #[command(name = "block-traces")]
    TraceBlockTransactions(TraceBlockTransactionsArg),
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct BlockIdArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetBlockArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block: BlockIdArg,

    /// Return block with receipts
    #[arg(long)]
    receipts: bool,

    /// Return only transaction hashes instead of full transactions
    #[arg(long, conflicts_with = "receipts")]
    tx_hashes_only: bool,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct TxHashArgs {
    /// Transaction hash
    #[arg(value_name = "TX_HASH")]
    tx_hash: TxHash,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetTransactionArgs {
    /// Transaction hash
    #[arg(value_name = "TX_HASH")]
    tx_hash: TxHash,

    /// Get only the transaction status instead of full transaction
    #[arg(long)]
    status: bool,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetStorageAtArgs {
    /// Contract address
    #[arg(value_name = "ADDRESS")]
    contract_address: ContractAddress,

    /// Storage key
    key: StorageKey,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(long)]
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetTransactionByBlockIdAndIndexArgs {
    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(default_value = "latest")]
    block: BlockIdArg,

    /// Transaction index
    index: u64,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetClassArgs {
    /// Class hash
    #[arg(value_name = "CLASS_HASH")]
    class_hash: ClassHash,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(long)]
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetClassHashAtArgs {
    /// Contract address
    #[arg(value_name = "ADDRESS")]
    contract_address: ContractAddress,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(long)]
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetClassAtArgs {
    /// Contract address
    #[arg(value_name = "ADDRESS")]
    contract_address: ContractAddress,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(long)]
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct CallArgs {
    /// Contract address
    #[arg(value_name = "ADDRESS")]
    contract_address: ContractAddress,

    /// Function selector
    selector: EntryPointSelector,

    /// Calldata (space-separated hex values)
    calldata: Vec<Felt>,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(long)]
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetEventsArgs {
    /// From block (number, hash, 'latest', or 'pending')
    #[arg(long)]
    from: Option<BlockIdArg>,

    /// To block (number, hash, 'latest', or 'pending')
    #[arg(long)]
    to: Option<BlockIdArg>,

    /// Contract address to filter events from
    #[arg(long)]
    address: Option<ContractAddress>,

    /// Event keys filter. Each key group is comma-separated, groups are space-separated.
    /// Example: --keys 0x1 0x2,0x3 represents [[0x1], [0x2, 0x3]]
    #[arg(long, num_args = 0..)]
    keys: Vec<String>,

    /// Continuation token from previous query for pagination
    #[arg(long)]
    continuation_token: Option<String>,

    /// Number of events to return per page
    #[arg(long, default_value = "10")]
    chunk_size: u64,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GetNonceArgs {
    /// The contract address whose nonce is requested
    #[arg(value_name = "ADDRESS")]
    address: ContractAddress,

    /// Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'
    #[arg(long)]
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
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
    #[arg(long)]
    #[arg(default_value = "latest")]
    block: BlockIdArg,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
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
                let block_id = args.block.0;

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
                let block_id = args.block.0;
                let result = client.get_state_update(block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::GetStorageAt(args) => {
                let contract_address = args.contract_address;
                let key = args.key;
                let block_id = args.block.0;
                let result = client.get_storage_at(contract_address, key, block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::GetTransactionByHash(args) => {
                let hash = args.tx_hash;

                if args.status {
                    let result = client.get_transaction_status(hash).await?;
                    println!("{}", colored_json::to_colored_json_auto(&result)?);
                } else {
                    let result = client.get_transaction_by_hash(hash).await?;
                    println!("{}", colored_json::to_colored_json_auto(&result)?);
                };
            }

            StarknetCommands::GetTransactionByBlockIdAndIndex(args) => {
                let block_id = args.block.0;
                let result =
                    client.get_transaction_by_block_id_and_index(block_id, args.index).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::GetTransactionReceipt(args) => {
                let tx_hash = args.tx_hash;
                let result = client.get_transaction_receipt(tx_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::GetClass(args) => {
                let block_id = args.block.0;
                let class_hash = args.class_hash;
                let result = client.get_class(block_id, class_hash).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::GetClassHashAt(args) => {
                let block_id = args.block.0;
                let contract_address = args.contract_address;
                let result = client.get_class_hash_at(block_id, contract_address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::GetClassAt(args) => {
                let block_id = args.block.0;
                let contract_address = args.contract_address;
                let result = client.get_class_at(block_id, contract_address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::GetBlockTransactionCount(args) => {
                let block_id = args.block.0;
                let result = client.get_block_transaction_count(block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::Call(args) => {
                let contract_address = args.contract_address;
                let entry_point_selector = args.selector;
                let calldata = args.calldata;

                let function_call =
                    FunctionCall { contract_address, entry_point_selector, calldata };

                let block_id = args.block.0;
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
                let block_id = args.block.0;
                let address = args.address;
                let result = client.get_nonce(block_id, address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::GetEvents(args) => {
                // Parse keys if provided
                let keys: Option<Vec<Vec<Felt>>> =
                    if !args.keys.is_empty() { Some(parse_event_keys(&args.keys)?) } else { None };

                let event_filter = EventFilter {
                    keys,
                    address: args.address,
                    to_block: args.to.map(|b| b.0),
                    from_block: args.from.map(|b| b.0),
                };

                let result_page_request = ResultPageRequest {
                    chunk_size: args.chunk_size,
                    continuation_token: args.continuation_token,
                };

                let filter = EventFilterWithPage { event_filter, result_page_request };

                let result = client.get_events(filter).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            StarknetCommands::TraceTransaction(args) => {
                let tx_hash = args.tx_hash;
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

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct BlockIdArg(pub BlockIdOrTag);

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct ConfirmedBlockIdArg(pub ConfirmedBlockIdOrTag);

impl std::str::FromStr for BlockIdArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let id = match s {
            "latest" => BlockIdOrTag::Latest,
            "l1_accepted" => BlockIdOrTag::L1Accepted,
            "pre_confirmed" | "preconfirmed" => BlockIdOrTag::PreConfirmed,

            hash if s.starts_with("0x") => BlockHash::from_hex(hash)
                .map(BlockIdOrTag::Hash)
                .with_context(|| format!("Invalid block hash: {hash}"))?,

            num => num
                .parse::<BlockNumber>()
                .map(BlockIdOrTag::Number)
                .with_context(|| format!("Invalid block number format: {num}"))?,
        };

        Ok(BlockIdArg(id))
    }
}

impl Default for BlockIdArg {
    fn default() -> Self {
        BlockIdArg(BlockIdOrTag::Latest)
    }
}

impl std::str::FromStr for ConfirmedBlockIdArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let id = match s {
            "latest" => ConfirmedBlockIdOrTag::Latest,
            "l1_accepted" => ConfirmedBlockIdOrTag::L1Accepted,

            hash if s.starts_with("0x") => BlockHash::from_hex(hash)
                .map(ConfirmedBlockIdOrTag::Hash)
                .with_context(|| format!("Invalid block hash: {hash}"))?,

            num => num
                .parse::<BlockNumber>()
                .map(ConfirmedBlockIdOrTag::Number)
                .with_context(|| format!("Invalid block number format: {num}"))?,
        };

        Ok(ConfirmedBlockIdArg(id))
    }
}

impl Default for ConfirmedBlockIdArg {
    fn default() -> Self {
        ConfirmedBlockIdArg(ConfirmedBlockIdOrTag::Latest)
    }
}

/// Parses event keys from CLI arguments.
///
/// Format: Each argument is a comma-separated list of felts.
/// Example: ["0x1", "0x2,0x3", "0x4"] => [[0x1], [0x2, 0x3], [0x4]]
fn parse_event_keys(keys: &[String]) -> Result<Vec<Vec<Felt>>> {
    keys.iter()
        .enumerate()
        .map(|(i, group)| {
            group
                .split(',')
                .map(|s| {
                    s.trim()
                        .parse::<Felt>()
                        .with_context(|| format!("invalid felt in key group {}: '{}'", i, s))
                })
                .collect::<Result<Vec<Felt>>>()
        })
        .collect()
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

    use clap::Parser;

    use super::{parse_event_keys, GetEventsArgs};

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        args: GetEventsArgs,
    }

    #[test]
    fn get_events_args_single_keys() {
        let args = TestCli::try_parse_from(["test", "--keys", "0x1", "0x2", "0x3"]).unwrap();

        let keys = parse_event_keys(&args.args.keys).unwrap();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0], vec![felt!("0x1")]);
        assert_eq!(keys[1], vec![felt!("0x2")]);
        assert_eq!(keys[2], vec![felt!("0x3")]);
    }

    #[test]
    fn get_events_args_multiple_keys() {
        let args =
            TestCli::try_parse_from(["test", "--keys", "0x9", "0x1,0x2,0x3", "0x4,0x5"]).unwrap();

        let keys = parse_event_keys(&args.args.keys).unwrap();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0], vec![felt!("0x9")]);
        assert_eq!(keys[1], vec![felt!("0x1"), felt!("0x2"), felt!("0x3")]);
        assert_eq!(keys[2], vec![felt!("0x4"), felt!("0x5")]);
    }

    #[test]
    fn get_events_args_keys_with_whitespace() {
        let args = TestCli::try_parse_from(["test", "--keys", "0x1, 0x2 , 0x3"]).unwrap();

        let keys = parse_event_keys(&args.args.keys).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], vec![felt!("0x1"), felt!("0x2"), felt!("0x3")]);
    }

    #[test]
    fn get_events_args_invalid_felt() {
        let args = TestCli::try_parse_from(["test", "--keys", "0x1", "invalid"]).unwrap();

        let result = parse_event_keys(&args.args.keys);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid felt in key group"));
    }

    #[test]
    fn get_events_args_invalid_hex() {
        let args = TestCli::try_parse_from(["test", "--keys", "0x1,0xGGG"]).unwrap();

        let result = parse_event_keys(&args.args.keys);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid felt in key group 0"));
    }
}
