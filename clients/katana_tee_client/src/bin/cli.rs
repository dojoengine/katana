//! Katana TEE CLI
//!
//! A command-line interface for interacting with Katana TEE attestations
//! and generating SP1 proofs.

use clap::{Parser, Subcommand};
use katana_tee_client::prover::{generate_sp1_proof_with_cache, generate_sp1_proof_with_config};
use katana_tee_client::starknet::{build_single_owner_account, KatanaTeeStarknetClient};
use katana_tee_client::{prover::verify_proof_structure, KatanaRpcClient, ProverConfig, TeeQuoteResponse};
use amd_tee_registry_client::{StarknetCalldata, StarknetRegistryClient};
use starknet_rust_accounts::ExecutionEncoding;
use starknet_rust_core::types::Felt;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "katana-tee")]
#[command(about = "Katana TEE Client - Fetch attestations and generate SP1 proofs")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch a TEE attestation quote from Katana RPC
    Fetch {
        /// Katana RPC URL
        #[arg(
            short,
            long,
            env = "KATANA_RPC_URL",
            default_value = "http://localhost:5050"
        )]
        rpc: String,

        /// Output file path (optional, prints to stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Execute the SP1 program in mock mode (fast, no real proof)
    Execute {
        /// Katana RPC URL (used if --json not specified)
        #[arg(
            short,
            long,
            env = "KATANA_RPC_URL",
            default_value = "http://localhost:5050"
        )]
        rpc: String,

        /// Load attestation from JSON file instead of RPC
        #[arg(short, long)]
        json: Option<PathBuf>,
    },

    /// Generate an SP1 Groth16 proof
    Prove {
        /// Katana RPC URL (used if --json not specified)
        #[arg(
            short,
            long,
            env = "KATANA_RPC_URL",
            default_value = "http://localhost:5050"
        )]
        rpc: String,

        /// Load attestation from JSON file instead of RPC
        #[arg(short, long)]
        json: Option<PathBuf>,

        /// Prover mode: mock, cpu, or network
        #[arg(short, long, env = "SP1_PROVER", default_value = "network")]
        prover: String,

        /// Override SP1 network private key (defaults to env `NETWORK_PRIVATE_KEY`, fallback `SP1_PRIVATE_KEY`)
        #[arg(long)]
        sp1_private_key: Option<String>,

        /// Override SP1 RPC URL (defaults to env `SP1_RPC_URL`)
        #[arg(long)]
        sp1_rpc_url: Option<String>,

        /// Skip certificate time validity check (defaults to env `SKIP_TIME_VALIDITY_CHECK`)
        #[arg(long)]
        skip_time_validity_check: bool,

        /// Output file for the proof
        #[arg(short, long, default_value = "proof_output.json")]
        output: PathBuf,
    },

    /// Full pipeline: fetch quote → query cache → prove → calldata → invoke `katana_tee`
    Pipeline {
        /// Katana RPC URL (also used as Starknet RPC if --starknet-rpc not provided)
        #[arg(
            short,
            long,
            env = "KATANA_RPC_URL",
            default_value = "http://localhost:5050"
        )]
        rpc: String,

        /// Load attestation from JSON file instead of RPC
        #[arg(short, long)]
        json: Option<PathBuf>,

        /// Starknet JSON-RPC URL (defaults to --rpc)
        #[arg(long, env = "STARKNET_RPC_URL")]
        starknet_rpc: Option<String>,

        /// Katana TEE contract address (hex felt)
        #[arg(long)]
        katana_tee: String,

        /// AMD TEE registry contract address (hex felt). If omitted, fetched from `katana_tee`.
        #[arg(long)]
        registry: Option<String>,

        /// Prover mode: mock, cpu, or network
        #[arg(short, long, env = "SP1_PROVER", default_value = "network")]
        prover: String,

        /// Override SP1 network private key (defaults to env `NETWORK_PRIVATE_KEY`, fallback `SP1_PRIVATE_KEY`)
        #[arg(long)]
        sp1_private_key: Option<String>,

        /// Override SP1 RPC URL (defaults to env `SP1_RPC_URL`)
        #[arg(long)]
        sp1_rpc_url: Option<String>,

        /// Skip certificate time validity check (defaults to env `SKIP_TIME_VALIDITY_CHECK`)
        #[arg(long)]
        skip_time_validity_check: bool,

        /// Skip on-chain cache lookup for trusted cert prefix length (use default value 2)
        #[arg(long)]
        skip_cache: bool,

        /// Output file for the proof JSON
        #[arg(long, default_value = "proof_output.json")]
        proof_output: PathBuf,

        /// Output file for the Starknet calldata (hex, newline-separated)
        #[arg(long)]
        calldata_output: Option<PathBuf>,

        /// Submit the Starknet transaction from this account address (hex felt)
        #[arg(long, env = "STARKNET_ACCOUNT_ADDRESS")]
        account_address: Option<String>,

        /// Submit the Starknet transaction using this account private key (hex felt)
        #[arg(long, env = "STARKNET_PRIVATE_KEY")]
        account_private_key: Option<String>,

        /// Account calldata encoding: `new` (Cairo 1) or `legacy` (Cairo 0)
        #[arg(long, default_value = "new", value_parser = ["new", "legacy"])]
        account_encoding: String,

        /// Do not submit transaction; only generate proof + calldata
        #[arg(long)]
        dry_run: bool,
    },

    /// Display information about a proof file
    Info {
        /// Path to the proof JSON file
        #[arg(default_value = "proof_output.json")]
        file: PathBuf,
    },

    /// Fetch AMD root certificates from KDS and output their hashes
    FetchRootCerts {
        /// Processor types to fetch (comma-separated: milan,genoa,bergamo,siena)
        #[arg(long, default_value = "milan,genoa")]
        processors: String,

        /// Output JSON file path (prints to stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Directory containing .der files to validate against
        #[arg(long)]
        validate: Option<PathBuf>,
    },

    /// Generate Cairo test fixtures from proof files
    GenerateCairoFixtures {
        /// Directory containing block_N subdirectories with proof.json files
        #[arg(long, default_value = "tests/fixtures")]
        fixture_dir: PathBuf,

        /// Output Cairo file path
        #[arg(short, long, default_value = "contracts/amd_tee_registry/tests/test_journal_decode_from_fixtures/test_journal_decode_fixtures.cairo")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("katana_tee_client=info".parse()?)
                .add_directive("amd_sev_snp_attestation_prover=info".parse()?),
        )
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    match cli.command {
        Commands::Fetch { rpc, output } => cmd_fetch(&rpc, output).await,
        Commands::Execute { rpc, json } => cmd_execute(&rpc, json).await,
        Commands::Prove {
            rpc,
            json,
            prover,
            sp1_private_key,
            sp1_rpc_url,
            skip_time_validity_check,
            output,
        } => {
            cmd_prove(
                &rpc,
                json,
                &prover,
                sp1_private_key,
                sp1_rpc_url,
                skip_time_validity_check,
                &output,
            )
            .await
        }
        Commands::Pipeline {
            rpc,
            json,
            starknet_rpc,
            katana_tee,
            registry,
            prover,
            sp1_private_key,
            sp1_rpc_url,
            skip_time_validity_check,
            skip_cache,
            proof_output,
            calldata_output,
            account_address,
            account_private_key,
            account_encoding,
            dry_run,
        } => {
            cmd_pipeline(
                &rpc,
                json,
                starknet_rpc,
                &katana_tee,
                registry.as_deref(),
                &prover,
                sp1_private_key,
                sp1_rpc_url,
                skip_time_validity_check,
                skip_cache,
                &proof_output,
                calldata_output,
                account_address,
                account_private_key,
                &account_encoding,
                dry_run,
            )
            .await
        }
        Commands::Info { file } => cmd_info(&file),
        Commands::FetchRootCerts { processors, output, validate } => {
            cmd_fetch_root_certs(&processors, output, validate)
        }
        Commands::GenerateCairoFixtures { fixture_dir, output } => {
            cmd_generate_cairo_fixtures(&fixture_dir, &output)
        }
    }
}

async fn cmd_fetch(rpc: &str, output: Option<PathBuf>) -> anyhow::Result<()> {
    println!("🌐 Fetching attestation from: {}", rpc);

    let client = KatanaRpcClient::new(rpc);
    let attestation = client.fetch_attestation().await?;

    println!("✅ Attestation received");
    println!();
    println!("📋 Details:");
    println!("   Block Number: {}", attestation.block_number);
    println!("   Block Hash:   {}", attestation.block_hash);
    println!("   State Root:   {}", attestation.state_root);
    println!(
        "   Quote Size:   {} bytes",
        attestation.quote_bytes()?.len()
    );

    if let Some(path) = output {
        let json = serde_json::to_string_pretty(&attestation)?;
        std::fs::write(&path, &json)?;
        println!();
        println!("💾 Saved to: {}", path.display());
    } else {
        println!();
        println!("📄 JSON:");
        println!("{}", serde_json::to_string_pretty(&attestation)?);
    }

    Ok(())
}

async fn cmd_execute(rpc: &str, json: Option<PathBuf>) -> anyhow::Result<()> {
    // Force mock mode
    std::env::set_var("SP1_PROVER", "mock");

    let attestation = get_attestation(rpc, json).await?;

    println!();
    println!("📋 Attestation:");
    println!("   Block Number: {}", attestation.block_number);
    println!(
        "   Quote Size:   {} bytes",
        attestation.quote_bytes()?.len()
    );
    println!();

    println!("🔄 Executing SP1 program (mock mode)...");
    let start = std::time::Instant::now();
    let proof = generate_sp1_proof_with_config(attestation, ProverConfig::default()).await?;
    let elapsed = start.elapsed();

    println!("✅ Execution completed in {:.2?}", elapsed);
    println!();
    println!("📊 Result:");
    println!("   Verifier ID: {}", proof.program_id.verifier_id);
    println!();
    println!("ℹ️  Mock mode - no real proof generated");

    Ok(())
}

async fn cmd_prove(
    rpc: &str,
    json: Option<PathBuf>,
    prover: &str,
    sp1_private_key: Option<String>,
    sp1_rpc_url: Option<String>,
    skip_time_validity_check: bool,
    output: &PathBuf,
) -> anyhow::Result<()> {
    std::env::set_var("SP1_PROVER", prover);

    let config = resolve_prover_config(sp1_private_key, sp1_rpc_url, skip_time_validity_check);

    // Validate network mode has key
    if prover == "network" {
        if !config.has_network_key() {
            anyhow::bail!(
                "NETWORK_PRIVATE_KEY required for network proving.\n\
                 Set it in .env or environment."
            );
        }

        // Note: The "insecure random number generator" warning during local execution
        // does NOT affect security. Local execution only determines cycle counts.
        // The actual proof is generated on the SP1 Network with secure randomness.
    }

    let attestation = get_attestation(rpc, json).await?;

    println!();
    println!("📋 Attestation:");
    println!("   Block Number: {}", attestation.block_number);
    println!("   Block Hash:   {}", attestation.block_hash);
    println!("   State Root:   {}", attestation.state_root);
    println!(
        "   Quote Size:   {} bytes",
        attestation.quote_bytes()?.len()
    );
    println!();

    println!("⚙️  Prover: {}", prover);
    if prover == "network" {
        println!("   This may take 1-2 minutes for Groth16 proving...");
    }
    println!();

    println!("🔄 Generating SP1 proof...");
    let start = std::time::Instant::now();
    let proof = generate_sp1_proof_with_config(attestation, config).await?;
    let elapsed = start.elapsed();

    println!("✅ Proof generated in {:.2?}", elapsed);
    println!();
    println!("📊 Proof Details:");
    println!("   ZK Type:      {:?}", proof.zktype);
    println!("   ZKVM Version: {}", proof.zkvm_version);
    println!("   Verifier ID:  {}", proof.program_id.verifier_id);
    println!("   Proof Size:   {} bytes", proof.onchain_proof.len());

    // Verify structure
    if prover != "mock" {
        verify_proof_structure(&proof)?;
        println!("   Verified:     ✓");
    }

    // Save proof
    let proof_json = proof.encode_json()?;
    std::fs::write(output, &proof_json)?;
    println!();
    println!("💾 Proof saved to: {}", output.display());

    Ok(())
}

fn cmd_info(file: &PathBuf) -> anyhow::Result<()> {
    println!("📄 Loading proof from: {}", file.display());

    let data = std::fs::read(file)?;
    let proof = amd_sev_snp_attestation_prover::OnchainProof::decode_json(&data)?;

    println!();
    println!("📊 Proof Details:");
    println!("   ZK Type:         {:?}", proof.zktype);
    println!("   ZKVM Version:    {}", proof.zkvm_version);
    println!("   Verifier ID:     {}", proof.program_id.verifier_id);
    println!("   Verify Proof ID: {}", proof.program_id.verify_proof_id);
    println!("   Onchain Proof:   {} bytes", proof.onchain_proof.len());

    // Show proof preview
    if !proof.onchain_proof.is_empty() {
        let hex = format!("{}", proof.onchain_proof);
        let preview_len = std::cmp::min(66, hex.len());
        println!("   Proof Preview:   {}...", &hex[..preview_len]);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_pipeline(
    rpc: &str,
    json: Option<PathBuf>,
    starknet_rpc: Option<String>,
    katana_tee: &str,
    registry: Option<&str>,
    prover: &str,
    sp1_private_key: Option<String>,
    sp1_rpc_url: Option<String>,
    skip_time_validity_check: bool,
    skip_cache: bool,
    proof_output: &PathBuf,
    calldata_output: Option<PathBuf>,
    account_address: Option<String>,
    account_private_key: Option<String>,
    account_encoding: &str,
    dry_run: bool,
) -> anyhow::Result<()> {
    let starknet_rpc = starknet_rpc.unwrap_or_else(|| rpc.to_string());

    std::env::set_var("SP1_PROVER", prover);
    let config = resolve_prover_config(sp1_private_key, sp1_rpc_url, skip_time_validity_check);

    if prover == "network" && !config.has_network_key() {
        anyhow::bail!(
            "NETWORK_PRIVATE_KEY required for network proving.\n\
             Set it in .env or environment, or pass --sp1-private-key."
        );
    }

    let attestation = get_attestation(rpc, json).await?;

    let state_root = felt_from_hex(&attestation.state_root)?;
    let block_hash = felt_from_hex(&attestation.block_hash)?;
    let block_number = attestation.block_number;

    println!("🔎 Starknet RPC: {}", starknet_rpc);
    println!("🧾 Katana block: {}", block_number);

    let katana_tee_addr = felt_from_hex(katana_tee)?;
    let katana_client = KatanaTeeStarknetClient::new(&starknet_rpc, katana_tee_addr)?;

    let registry_addr = match registry {
        Some(addr) => felt_from_hex(addr)?,
        None => {
            println!("🔎 Fetching registry address from katana_tee...");
            katana_client.get_registry_address().await?
        }
    };

    println!("🏛️  Registry: 0x{:x}", registry_addr);
    println!("🏛️  KatanaTee: 0x{:x}", katana_client.contract_address());

    let proof = if skip_cache {
        println!("🔄 Proving (skipping on-chain cache)...");
        let start = std::time::Instant::now();
        let proof = generate_sp1_proof_with_config(attestation, config).await?;
        let elapsed = start.elapsed();
        println!("✅ Proof generated in {:.2?}", elapsed);
        proof
    } else {
        let registry_client = StarknetRegistryClient::new(&starknet_rpc, registry_addr);
        println!("🔄 Proving (with on-chain cache)...");
        let start = std::time::Instant::now();
        let proof = generate_sp1_proof_with_cache(attestation, config, &registry_client).await?;
        let elapsed = start.elapsed();
        println!("✅ Proof generated in {:.2?}", elapsed);
        proof
    };

    // Save proof JSON
    let proof_json = proof.encode_json()?;
    std::fs::write(proof_output, &proof_json)?;
    println!("💾 Proof saved to: {}", proof_output.display());

    // Convert to Starknet calldata (Garaga)
    let calldata = StarknetCalldata::from_proof(&proof)?;
    println!("📦 Calldata elements: {}", calldata.len());

    if let Some(path) = calldata_output.as_ref() {
        calldata.save_to_file(path)?;
        println!("💾 Calldata saved to: {}", path.display());
    }

    // Verify structure (skip when mock produced empty proof bytes)
    if prover != "mock" {
        verify_proof_structure(&proof)?;
    }

    if dry_run {
        println!("🧪 Dry run: not submitting transaction.");
        return Ok(());
    }

    let account_address = account_address.ok_or_else(|| {
        anyhow::anyhow!("--account-address (or STARKNET_ACCOUNT_ADDRESS) is required unless --dry-run")
    })?;
    let account_private_key = account_private_key.ok_or_else(|| {
        anyhow::anyhow!(
            "--account-private-key (or STARKNET_PRIVATE_KEY) is required unless --dry-run"
        )
    })?;

    let encoding = match account_encoding {
        "new" => ExecutionEncoding::New,
        "legacy" => ExecutionEncoding::Legacy,
        _ => anyhow::bail!("Invalid --account-encoding (expected: new | legacy)"),
    };

    let account = build_single_owner_account(
        &starknet_rpc,
        felt_from_hex(&account_address)?,
        felt_from_hex(&account_private_key)?,
        encoding,
    )
    .await?;

    let sp1_proof = calldata.to_felts()?;

    println!("📤 Submitting verify_and_update_state...");
    let tx_hash = katana_client
        .verify_and_update_state(&account, sp1_proof, state_root, block_hash, block_number)
        .await?;

    println!("✅ Submitted. Tx hash: 0x{:x}", tx_hash);
    Ok(())
}

async fn get_attestation(rpc: &str, json: Option<PathBuf>) -> anyhow::Result<TeeQuoteResponse> {
    match json {
        Some(path) => {
            println!("📄 Loading attestation from: {}", path.display());
            Ok(TeeQuoteResponse::from_json_file(&path)?)
        }
        None => {
            println!("🌐 Fetching attestation from: {}", rpc);
            let client = KatanaRpcClient::new(rpc);
            Ok(client.fetch_attestation().await?)
        }
    }
}

fn resolve_prover_config(
    sp1_private_key: Option<String>,
    sp1_rpc_url: Option<String>,
    skip_time_validity_check: bool,
) -> ProverConfig {
    let private_key = sp1_private_key
        .or_else(|| std::env::var("NETWORK_PRIVATE_KEY").ok())
        .or_else(|| std::env::var("SP1_PRIVATE_KEY").ok());

    let rpc_url = sp1_rpc_url.or_else(|| std::env::var("SP1_RPC_URL").ok());

    let skip = if skip_time_validity_check {
        true
    } else {
        std::env::var("SKIP_TIME_VALIDITY_CHECK")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(false)
    };

    ProverConfig::new(private_key, rpc_url, skip)
}

fn cmd_fetch_root_certs(
    processors: &str,
    output: Option<PathBuf>,
    validate: Option<PathBuf>,
) -> anyhow::Result<()> {
    use amd_tee_registry_client::{KdsClient, parse_processor_type};

    let kds = KdsClient::new();
    let mut results = serde_json::Map::new();

    for proc_str in processors.split(',') {
        let proc_str = proc_str.trim();
        let proc_type = match parse_processor_type(proc_str) {
            Some(p) => p,
            None => {
                eprintln!("Unknown processor type: {}", proc_str);
                continue;
            }
        };

        println!("Fetching cert chain for {}...", proc_str);

        match kds.fetch_root_cert_hash(proc_type) {
            Ok(info) => {
                println!("  ARK hash: {}", info.ark_hash);

                // Validate against local .der file if provided
                if let Some(validate_dir) = &validate {
                    let der_path = validate_dir.join(format!("ark-{}.der", proc_str.to_lowercase()));
                    if der_path.exists() {
                        match kds.validate_against_file(proc_type, &der_path) {
                            Ok(true) => println!("  Matches local {}", der_path.display()),
                            Ok(false) => eprintln!("  MISMATCH with local {}!", der_path.display()),
                            Err(e) => eprintln!("  Validation error: {}", e),
                        }
                    }
                }

                let mut entry = serde_json::Map::new();
                entry.insert("ark_hash".to_string(), serde_json::Value::String(info.ark_hash));
                entry.insert("source".to_string(), serde_json::Value::String(info.source));
                results.insert(proc_str.to_lowercase(), serde_json::Value::Object(entry));
            }
            Err(e) => {
                eprintln!("  Error fetching {}: {}", proc_str, e);
            }
        }
    }

    let json_output = serde_json::to_string_pretty(&results)
        .expect("Failed to serialize JSON");

    if let Some(output_path) = output {
        std::fs::write(&output_path, &json_output)
            .expect("Failed to write output file");
        println!("\nSaved to {}", output_path.display());
    } else {
        println!("\n{}", json_output);
    }

    Ok(())
}

fn cmd_generate_cairo_fixtures(fixture_dir: &PathBuf, output: &PathBuf) -> anyhow::Result<()> {
    use amd_tee_registry_client::generate_cairo_fixtures;

    println!("Generating Cairo test fixtures...");
    println!("  Fixture dir: {}", fixture_dir.display());
    println!("  Output: {}", output.display());

    generate_cairo_fixtures(fixture_dir, output)?;

    println!("Cairo fixtures generated successfully!");
    Ok(())
}

fn felt_from_hex(value: &str) -> anyhow::Result<Felt> {
    let value = value.trim();
    let value = if value.starts_with("0x") || value.starts_with("0X") {
        value.to_string()
    } else {
        format!("0x{value}")
    };
    Ok(Felt::from_hex(&value)?)
}
