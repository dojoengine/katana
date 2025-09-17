use std::str::FromStr;

use clap::{Parser, Subcommand};
use katana_feeder_gateway::client::{Error, SequencerGateway};
use katana_feeder_gateway::types::BlockId;
use katana_primitives::Felt;

#[derive(Parser)]
#[command(name = "feeder-cli")]
#[command(about = "CLI tool for testing Feeder Gateway client", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The feeder gateway URL (defaults to Starknet sepolia)
    #[arg(long, value_name = "URL")]
    gateway_url: Option<String>,

    /// API key for bypassing rate limiting
    #[arg(long, env = "STARKNET_API_KEY")]
    api_key: Option<String>,

    /// Use mainnet instead of sepolia
    #[arg(long)]
    mainnet: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Get block information
    Block {
        /// Block identifier: number, hash (0x-prefixed), or "latest"
        #[arg(default_value = "latest")]
        block_id: String,
    },
    /// Get state update
    StateUpdate {
        /// Block identifier: number, hash (0x-prefixed), or "latest"
        #[arg(default_value = "latest")]
        block_id: String,

        /// Include block data in the response
        #[arg(long)]
        include_block: bool,
    },
    /// Get contract class by hash
    Class {
        /// Class hash (0x-prefixed)
        class_hash: String,

        /// Block identifier: number, hash (0x-prefixed), or "latest"
        #[arg(default_value = "latest")]
        block_id: String,
    },
    /// Get compiled class by hash
    CompiledClass {
        /// Class hash (0x-prefixed)
        class_hash: String,

        /// Block identifier: number, hash (0x-prefixed), or "latest"
        #[arg(default_value = "latest")]
        block_id: String,
    },
}

fn parse_block_id(s: &str) -> Result<BlockId, String> {
    if s == "latest" {
        return Ok(BlockId::Latest);
    }

    // Try to parse as number first
    if let Ok(num) = s.parse::<u64>() {
        return Ok(BlockId::Number(num));
    }

    // Try to parse as hex hash
    if s.starts_with("0x") || s.starts_with("0X") {
        match Felt::from_str(s) {
            Ok(hash) => Ok(BlockId::Hash(hash)),
            Err(e) => Err(format!("Invalid block hash: {}", e)),
        }
    } else {
        Err(format!("Invalid block identifier: {}. Use a number, 0x-prefixed hash, or 'latest'", s))
    }
}

fn parse_felt(s: &str) -> Result<Felt, String> {
    Felt::from_str(s).map_err(|e| format!("Invalid felt value: {}", e))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize the client
    let mut client = if let Some(url) = cli.gateway_url {
        let parsed_url = url::Url::parse(&url)?;
        SequencerGateway::new(parsed_url)
    } else if cli.mainnet {
        SequencerGateway::sn_mainnet()
    } else {
        SequencerGateway::sn_sepolia()
    };

    // Set API key if provided
    if let Some(api_key) = cli.api_key {
        client = client.with_api_key(api_key);
    }

    // Execute the command
    match cli.command {
        Commands::Block { block_id } => {
            let block_id = parse_block_id(&block_id)?;
            println!("Fetching block {:?}...", block_id);

            match client.get_block(block_id).await {
                Ok(block) => {
                    println!("Block retrieved successfully!");
                    println!("=====================================");
                    if let Some(hash) = block.block_hash {
                        println!("Block Hash: {:#x}", hash);
                    }
                    if let Some(number) = block.block_number {
                        println!("Block Number: {}", number);
                    }
                    println!("Parent Hash: {:#x}", block.parent_block_hash);
                    println!("Timestamp: {}", block.timestamp);
                    println!("Status: {:?}", block.status);
                    println!("Transaction Count: {}", block.transactions.len());
                    println!("L1 DA Mode: {:?}", block.l1_da_mode);
                    if let Some(version) = block.starknet_version {
                        println!("Starknet Version: {}", version);
                    }
                    if let Some(sequencer) = block.sequencer_address {
                        println!("Sequencer Address: {:#x}", sequencer.0);
                    }
                }
                Err(e) => handle_error(e),
            }
        }
        Commands::StateUpdate { block_id, include_block } => {
            let block_id = parse_block_id(&block_id)?;

            if include_block {
                println!("Fetching state update with block for {:?}...", block_id);
                match client.get_state_update_with_block(block_id).await {
                    Ok(result) => {
                        println!("State update retrieved successfully!");
                        println!("=====================================");
                        println!("Block Info:");
                        if let Some(hash) = result.block.block_hash {
                            println!("  Block Hash: {:#x}", hash);
                        }
                        if let Some(number) = result.block.block_number {
                            println!("  Block Number: {}", number);
                        }
                        println!("\nState Update:");
                        print_state_update(&result.state_update);
                    }
                    Err(e) => handle_error(e),
                }
            } else {
                println!("Fetching state update for {:?}...", block_id);
                match client.get_state_update(block_id).await {
                    Ok(state_update) => {
                        println!("State update retrieved successfully!");
                        println!("=====================================");
                        print_state_update(&state_update);
                    }
                    Err(e) => handle_error(e),
                }
            }
        }
        Commands::Class { class_hash, block_id } => {
            let hash = parse_felt(&class_hash)?;
            let block_id = parse_block_id(&block_id)?;
            println!("Fetching class {:#x} at block {:?}...", hash, block_id);

            match client.get_class(hash, block_id).await {
                Ok(class) => {
                    println!("Class retrieved successfully!");
                    println!("=====================================");
                    match class {
                        katana_feeder_gateway::types::ContractClass::Sierra(sierra) => {
                            println!("Type: Sierra Contract Class");
                            println!("Contract Name: {}", sierra.contract_class_version);
                            println!("Entry Points:");
                            println!(
                                "  External: {} functions",
                                sierra.entry_points_by_type.external.len()
                            );
                            println!(
                                "  L1 Handler: {} functions",
                                sierra.entry_points_by_type.l1_handler.len()
                            );
                            println!(
                                "  Constructor: {} functions",
                                sierra.entry_points_by_type.constructor.len()
                            );
                        }
                        katana_feeder_gateway::types::ContractClass::Legacy(_) => {
                            println!("Type: Legacy Contract Class");
                        }
                    }
                }
                Err(e) => handle_error(e),
            }
        }
        Commands::CompiledClass { class_hash, block_id } => {
            let hash = parse_felt(&class_hash)?;
            let block_id = parse_block_id(&block_id)?;
            println!("Fetching compiled class {:#x} at block {:?}...", hash, block_id);

            match client.get_compiled_class(hash, block_id).await {
                Ok(compiled_class) => {
                    println!("Compiled class retrieved successfully!");
                    println!("=====================================");
                    println!("Compiler Version: {}", compiled_class.compiler_version);
                    println!("Bytecode Length: {}", compiled_class.bytecode.len());
                    println!("Hints: {} entries", compiled_class.hints.len());
                    println!("Entry Points:");
                    println!(
                        "  External: {} functions",
                        compiled_class.entry_points_by_type.external.len()
                    );
                    println!(
                        "  L1 Handler: {} functions",
                        compiled_class.entry_points_by_type.l1_handler.len()
                    );
                    println!(
                        "  Constructor: {} functions",
                        compiled_class.entry_points_by_type.constructor.len()
                    );
                }
                Err(e) => handle_error(e),
            }
        }
    }

    Ok(())
}

fn print_state_update(state_update: &katana_feeder_gateway::types::StateUpdate) {
    if let Some(block_hash) = state_update.block_hash {
        println!("  Block Hash: {:#x}", block_hash);
    }
    if let Some(new_root) = state_update.new_root {
        println!("  New Root: {:#x}", new_root);
    }
    println!("  Old Root: {:#x}", state_update.old_root);
    println!("  State Diff:");
    println!("    Storage Diffs: {} contracts", state_update.state_diff.storage_diffs.len());
    println!("    Deployed Contracts: {}", state_update.state_diff.deployed_contracts.len());
    println!("    Declared Classes: {}", state_update.state_diff.declared_classes.len());
    println!(
        "    Old Declared Contracts: {}",
        state_update.state_diff.old_declared_contracts.len()
    );
    println!("    Nonces: {} updated", state_update.state_diff.nonces.len());
    println!("    Replaced Classes: {}", state_update.state_diff.replaced_classes.len());
}

fn handle_error(error: Error) {
    eprintln!("Error: {}", error);

    match error {
        Error::RateLimited => {
            eprintln!("\nYou've been rate limited. Consider:");
            eprintln!("  - Setting the STARKNET_API_KEY environment variable");
            eprintln!("  - Using the --api-key flag");
            eprintln!("  - Waiting a moment before retrying");
        }
        Error::Sequencer(ref seq_err) => {
            eprintln!("\nSequencer error code: {:?}", seq_err.code);
        }
        _ => {}
    }

    std::process::exit(1);
}
