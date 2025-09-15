use anyhow::Result;
use katana_db::version::CURRENT_DB_VERSION;
use katana_node_bindings::Katana;
use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::{felt, ContractAddress};
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_client::HttpClientBuilder;
use katana_rpc_types::{EventFilter, FunctionCall};

#[tokio::main]
async fn main() -> Result<()> {
    println!("Testing database compatibility from version 1.6.0");
    println!("Current Katana database version: {CURRENT_DB_VERSION}");

    const TEST_DB_DIR: &str = "tests/fixtures/db/db_v1.6.0";

    // Get the katana binary path from the environment variable if provided (for CI)
    // Otherwise, assume it's in PATH
    let katana = if let Ok(binary_path) = std::env::var("KATANA_BIN") {
        Katana::at(binary_path)
    } else {
        Katana::new()
    };

    let instance = katana.db_dir(TEST_DB_DIR).spawn();

    // Create HTTP client for Katana's RPC
    let url = format!("http://{}", instance.rpc_addr());
    println!("Katana RPC URL: {}", url);

    // Give the node some time to fully initialize
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let http_client = HttpClientBuilder::default().build(&url)?;
    let client = StarknetClient::new(http_client);

    println!("\n=== Testing Block-related Tables ===");
    println!("Tables: Headers, BlockHashes, BlockNumbers, BlockBodyIndices, BlockStatusses");

    // Test block-related queries that access Headers, BlockHashes, BlockNumbers tables
    let latest_block_number = client.block_number().await?;
    println!("✓ Latest block number retrieved: {:?}", latest_block_number);

    // Get block by number (accesses Headers, BlockBodyIndices)
    // Try block 0 first since latest block number is 0
    let block_0 = client
        .get_block_with_txs(BlockIdOrTag::Number(0))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get block 0: {}", e))?;
    let block_1_hash = match &block_0 {
        katana_rpc_types::block::MaybePreConfirmedBlock::Confirmed(b) => b.block_hash,
        katana_rpc_types::block::MaybePreConfirmedBlock::PreConfirmed(b) => {
            // PreConfirmed blocks might not have block_hash field, use a default
            katana_primitives::block::BlockHash::default()
        }
    };
    println!("✓ Block 0 hash: {:#x}", block_1_hash);

    // Get block by hash (accesses BlockNumbers, Headers)
    let block_by_hash = client.get_block_with_txs(BlockIdOrTag::Hash(block_1_hash)).await?;
    let block_number = match block_by_hash {
        katana_rpc_types::block::MaybePreConfirmedBlock::Confirmed(b) => b.block_number,
        katana_rpc_types::block::MaybePreConfirmedBlock::PreConfirmed(b) => {
            // PreConfirmed blocks might have block_number
            0u64
        }
    };
    println!("✓ Block by hash retrieved: block #{}", block_number);

    // Get block with tx hashes (accesses BlockBodyIndices)
    let block_with_tx_hashes = client.get_block_with_tx_hashes(BlockIdOrTag::Number(27)).await?;
    let tx_count = match block_with_tx_hashes {
        katana_rpc_types::block::GetBlockWithTxHashesResponse::Block(b) => b.transactions.len(),
        katana_rpc_types::block::GetBlockWithTxHashesResponse::PreConfirmed(b) => {
            b.transactions.len()
        }
    };
    println!("✓ Block 27 has {} transactions", tx_count);

    println!("\n=== Testing Transaction Tables ===");
    println!("Tables: TxNumbers, TxBlocks, TxHashes, TxTraces, Transactions, Receipts");

    // Test transaction-related queries
    // Transaction from block 1 (DECLARE transaction)
    let tx_hash_declare = felt!("0x241f540802ddd4eefc1b7513b630c30fc684b304b97eb0c03af625e2dffd12");

    // Get transaction by hash (accesses TxNumbers, Transactions)
    let tx = client.get_transaction_by_hash(tx_hash_declare).await?;
    println!("✓ Transaction found");

    // Get transaction receipt (accesses Receipts, TxBlocks)
    let receipt = client.get_transaction_receipt(tx_hash_declare).await?;
    println!("✓ Transaction receipt retrieved, status: {:?}", receipt.receipt.finality_status());

    // Get transaction by block ID and index (accesses TxHashes, Transactions)
    let tx_by_index =
        client.get_transaction_by_block_id_and_index(BlockIdOrTag::Number(1), 0).await?;
    println!("✓ Transaction by index retrieved");

    // Transaction from block 27 (INVOKE transaction)
    let tx_hash_invoke = felt!("0x722b80b0fd8e4cd177bb565ee73843ce5ffb8cc07114207cd4399cb4e00c9ac");

    // Get trace (accesses TxTraces)
    let trace = client.trace_transaction(tx_hash_invoke).await?;
    println!("✓ Transaction trace retrieved for invoke tx");

    // Get block traces (accesses TxTraces for all txs in block)
    let block_traces = client.trace_block_transactions(ConfirmedBlockIdOrTag::Number(1)).await?;
    // TraceBlockTransactionsResponse is likely a vector or has a field with traces
    println!("✓ Block traces retrieved successfully");

    println!("\n=== Testing Class and Contract Tables ===");
    println!(
        "Tables: CompiledClassHashes, Classes, ContractInfo, ClassDeclarationBlock, \
         ClassDeclarations"
    );

    // Class hash from the DECLARE transaction
    let class_hash = felt!("0x685ed02eefa98fe7e208aa295042e9bbad8029b0d3d6f0ba2b32546efe0a1f9");

    // Get class (accesses Classes, CompiledClassHashes)
    let class = client.get_class(BlockIdOrTag::Latest, class_hash).await?;
    println!("✓ Class definition retrieved");

    // Account address from the test data
    let account_address =
        ContractAddress(felt!("0x2af9427c5a277474c079a1283c880ee8a6f0f8fbf73ce969c08d88befec1bba"));

    // Get class at address (accesses ContractInfo)
    let class_at = client.get_class_at(BlockIdOrTag::Latest, account_address).await?;
    println!("✓ Class at address retrieved");

    // Get class hash at address (accesses ContractInfo)
    let class_hash_at = client.get_class_hash_at(BlockIdOrTag::Latest, account_address).await?;
    println!("✓ Class hash at address: {:#x}", class_hash_at);

    // Get nonce (accesses ContractInfo, NonceChangeHistory)
    let nonce = client.get_nonce(BlockIdOrTag::Latest, account_address).await?;
    println!("✓ Account nonce: {:#x}", nonce);

    println!("\n=== Testing Storage Tables ===");
    println!("Tables: ContractStorage, StorageChangeHistory, StorageChangeSet");

    // Storage key for balance in ERC20 contract
    let storage_key = felt!("0x0");

    // Get storage at (accesses ContractStorage, StorageChangeHistory)
    let eth_token =
        ContractAddress(felt!("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"));
    let storage_value = client.get_storage_at(eth_token, storage_key, BlockIdOrTag::Latest).await?;
    println!("✓ Storage value at ETH token: {:#x}", storage_value);

    println!("\n=== Testing State Update Tables ===");
    println!("Tables: ContractInfoChangeSet, NonceChangeHistory, ClassChangeHistory, Trie tables");

    // Get state update (accesses multiple change history tables and trie tables)
    let state_update = client.get_state_update(BlockIdOrTag::Number(1)).await?;
    println!("✓ State update for block 1 retrieved");

    // Get state update for latest block (accesses latest state)
    let latest_state_update = client.get_state_update(BlockIdOrTag::Latest).await?;
    println!("✓ State update for latest block retrieved");

    println!("\n=== Testing Additional RPC Methods ===");

    // Call contract (reads from ContractInfo, ContractStorage, and executes)
    let call_result = client
        .call(
            FunctionCall {
                contract_address: eth_token,
                entry_point_selector: felt!(
                    "0x35a73cd311a05d46deda634c5ee045db92f811b4e74bca4437fcb5302b7af33"
                ), // decimals()
                calldata: vec![],
            },
            BlockIdOrTag::Latest,
        )
        .await?;
    println!("✓ Contract call executed successfully");

    // Get events (accesses Receipts and filters events)
    let events = client
        .get_events(
            EventFilter {
                from_block: Some(BlockIdOrTag::Number(0)),
                to_block: Some(BlockIdOrTag::Number(5)),
                address: None,
                keys: None,
            },
            None,
            1000,
        )
        .await?;
    println!("✓ Events retrieved: {} events found", events.events.len());

    // Get block transaction count (accesses BlockBodyIndices)
    let tx_count = client.get_block_transaction_count(BlockIdOrTag::Number(27)).await?;
    println!("✓ Transaction count in block 27: {}", tx_count);

    // Get chain ID
    let chain_id = client.chain_id().await?;
    println!("✓ Chain ID: {:#x}", chain_id);

    // Get syncing status
    let syncing = client.syncing().await?;
    println!("✓ Syncing status: {:?}", syncing);

    println!("\n=== Database Compatibility Test PASSED ===");
    println!(
        "Successfully initialized Katana with v1.6.0 database and exercised all major database \
         tables!"
    );

    Ok(())
}
