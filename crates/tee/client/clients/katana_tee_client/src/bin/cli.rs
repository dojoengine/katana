//! Katana TEE CLI
//!
//! A command-line interface for interacting with Katana TEE attestations
//! and generating SP1 proofs.

use clap::{Parser, Subcommand};
use katana_tee_client::starknet::{build_single_owner_account, KatanaTeeStarknetClient};
use katana_tee_client::{
    AmdAttestationProver, KatanaRpcClient, ProverConfig, StarknetCalldata, StarknetRegistryClient,
    TeeQuoteResponse,
};
use starknet_rust_accounts::ExecutionEncoding;
use starknet_rust_core::types::Felt;
use std::path::{Path, PathBuf};

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

        /// Starknet JSON-RPC URL for cache lookup (defaults to --rpc)
        #[arg(long, env = "STARKNET_RPC_URL")]
        starknet_rpc: Option<String>,

        /// AMD TEE registry contract address (hex felt)
        #[arg(long, required = true)]
        registry: String,
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

        /// Starknet JSON-RPC URL for cache lookup (defaults to --rpc)
        #[arg(long, env = "STARKNET_RPC_URL")]
        starknet_rpc: Option<String>,

        /// AMD TEE registry contract address (hex felt)
        #[arg(long, required = true)]
        registry: String,

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

        /// AMD TEE registry contract address (hex felt)
        #[arg(long, required = true)]
        registry: String,

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

        /// Output file for the proof JSON
        #[arg(long, default_value = "proof_output.json")]
        proof_output: PathBuf,

        /// Use existing proof file instead of generating (skips SP1 proving)
        #[arg(long)]
        proof_input: Option<PathBuf>,

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
        #[arg(
            short,
            long,
            default_value = "contracts/amd_tee_registry/tests/test_journal_decode_from_fixtures/test_journal_decode_fixtures.cairo"
        )]
        output: PathBuf,
    },

    /// Generate Starknet calldata from a proof file
    Calldata {
        /// Path to the proof JSON file
        #[arg(short, long, default_value = "proof_output.json")]
        proof: PathBuf,

        /// Output file for calldata (prints to stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,
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
        Commands::Execute {
            rpc,
            json,
            starknet_rpc,
            registry,
        } => cmd_execute(&rpc, json, starknet_rpc, &registry).await,
        Commands::Prove {
            rpc,
            json,
            starknet_rpc,
            registry,
            prover,
            sp1_private_key,
            sp1_rpc_url,
            skip_time_validity_check,
            output,
        } => {
            cmd_prove(
                &rpc,
                json,
                starknet_rpc,
                &registry,
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
            proof_output,
            proof_input,
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
                &registry,
                &prover,
                sp1_private_key,
                sp1_rpc_url,
                skip_time_validity_check,
                &proof_output,
                proof_input,
                calldata_output,
                account_address,
                account_private_key,
                &account_encoding,
                dry_run,
            )
            .await
        }
        Commands::Info { file } => cmd_info(&file),
        Commands::FetchRootCerts {
            processors,
            output,
            validate,
        } => cmd_fetch_root_certs(&processors, output, validate),
        Commands::GenerateCairoFixtures {
            fixture_dir,
            output,
        } => cmd_generate_cairo_fixtures(&fixture_dir, &output),
        Commands::Calldata { proof, output } => cmd_calldata(&proof, output),
    }
}

async fn cmd_fetch(rpc: &str, output: Option<PathBuf>) -> anyhow::Result<()> {
    println!("🌐 Fetching attestation from: {}", rpc);

    let client = KatanaRpcClient::new(rpc);
    let attestation = client.fetch_attestation(None, 0).await?;

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

async fn cmd_execute(
    rpc: &str,
    json: Option<PathBuf>,
    starknet_rpc: Option<String>,
    registry: &str,
) -> anyhow::Result<()> {
    let starknet_rpc = starknet_rpc.unwrap_or_else(|| rpc.to_string());

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

    let registry_addr = felt_from_hex(registry)?;
    let registry_client = StarknetRegistryClient::new(&starknet_rpc, registry_addr);

    println!("🏛️  Registry: 0x{:x}", registry_addr);
    println!("🔄 Executing SP1 program (mock mode)...");
    let start = std::time::Instant::now();
    let prover = AmdAttestationProver::new(ProverConfig::default());
    let proof_info = prover
        .prove(&attestation.quote_bytes()?, &registry_client)
        .await?;
    let elapsed = start.elapsed();

    println!("✅ Execution completed in {:.2?}", elapsed);
    println!("   Trusted prefix len: {}", proof_info.trusted_prefix_len);
    println!();
    println!("📊 Result:");
    println!(
        "   Verifier ID: {}",
        proof_info.proof.program_id.verifier_id
    );
    println!();
    println!("ℹ️  Mock mode - no real proof generated");

    Ok(())
}

async fn cmd_prove(
    rpc: &str,
    json: Option<PathBuf>,
    starknet_rpc: Option<String>,
    registry: &str,
    prover: &str,
    sp1_private_key: Option<String>,
    sp1_rpc_url: Option<String>,
    skip_time_validity_check: bool,
    output: &PathBuf,
) -> anyhow::Result<()> {
    let starknet_rpc = starknet_rpc.unwrap_or_else(|| rpc.to_string());

    std::env::set_var("SP1_PROVER", prover);

    let config = resolve_prover_config(sp1_private_key, sp1_rpc_url, skip_time_validity_check);

    // Validate network mode has key
    // Note: The "insecure random number generator" warning during local execution
    // does NOT affect security. Local execution only determines cycle counts.
    // The actual proof is generated on the SP1 Network with secure randomness.
    if prover == "network" && !config.has_network_key() {
        anyhow::bail!(
            "NETWORK_PRIVATE_KEY required for network proving.\n\
             Set it in .env or environment."
        );
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

    let registry_addr = felt_from_hex(registry)?;
    let registry_client = StarknetRegistryClient::new(&starknet_rpc, registry_addr);

    println!("🏛️  Registry: 0x{:x}", registry_addr);
    println!("⚙️  Prover: {}", prover);
    if prover == "network" {
        println!("   This may take 1-2 minutes for Groth16 proving...");
    }
    println!();

    println!("🔄 Generating SP1 proof (querying on-chain cache)...");
    let start = std::time::Instant::now();
    let amd_prover = AmdAttestationProver::new(config);
    let proof_info = amd_prover
        .prove(&attestation.quote_bytes()?, &registry_client)
        .await?;
    let elapsed = start.elapsed();

    let proof = &proof_info.proof;

    println!("✅ Proof generated in {:.2?}", elapsed);
    println!("   Trusted prefix len: {}", proof_info.trusted_prefix_len);
    println!("   Cert chain length: {}", proof_info.cert_digests.len());
    println!();
    println!("📊 Proof Details:");
    println!("   ZK Type:      {:?}", proof.zktype);
    println!("   ZKVM Version: {}", proof.zkvm_version);
    println!("   Verifier ID:  {}", proof.program_id.verifier_id);
    println!("   Proof Size:   {} bytes", proof.onchain_proof.len());

    // Verify structure
    if prover != "mock" {
        AmdAttestationProver::<katana_tee_client::Sp1NetworkBackend>::verify_proof_structure(
            proof,
        )?;
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
    registry: &str,
    prover: &str,
    sp1_private_key: Option<String>,
    sp1_rpc_url: Option<String>,
    skip_time_validity_check: bool,
    proof_output: &PathBuf,
    proof_input: Option<PathBuf>,
    calldata_output: Option<PathBuf>,
    account_address: Option<String>,
    account_private_key: Option<String>,
    account_encoding: &str,
    dry_run: bool,
) -> anyhow::Result<()> {
    let starknet_rpc = starknet_rpc.unwrap_or_else(|| rpc.to_string());

    let attestation = get_attestation(rpc, json).await?;

    let state_root = felt_from_hex(&attestation.state_root)?;
    let block_hash = felt_from_hex(&attestation.block_hash)?;
    let block_number = attestation.clone().block_number;

    println!("🔎 Starknet RPC: {}", starknet_rpc);
    println!("🧾 Katana block: {}", block_number);

    let katana_tee_addr = felt_from_hex(katana_tee)?;
    let katana_client = KatanaTeeStarknetClient::new(&starknet_rpc, katana_tee_addr)?;

    let registry_addr = felt_from_hex(registry)?;

    println!("🏛️  Registry: 0x{:x}", registry_addr);
    println!("🏛️  KatanaTee: 0x{:x}", katana_client.contract_address());

    // Either load existing proof or generate new one
    let proof = if let Some(proof_path) = proof_input {
        println!("📄 Loading existing proof from: {}", proof_path.display());
        let data = std::fs::read(&proof_path)?;
        let proof = amd_sev_snp_attestation_prover::OnchainProof::decode_json(&data)?;
        println!("✅ Proof loaded (skipping SP1 proving)");
        proof
    } else {
        std::env::set_var("SP1_PROVER", prover);
        let config = resolve_prover_config(sp1_private_key, sp1_rpc_url, skip_time_validity_check);

        if prover == "network" && !config.has_network_key() {
            anyhow::bail!(
                "NETWORK_PRIVATE_KEY required for network proving.\n\
                 Set it in .env or environment, or pass --sp1-private-key."
            );
        }

        let amd_prover = AmdAttestationProver::new(config);
        let quote_bytes = attestation.quote_bytes()?;

        let registry_client = StarknetRegistryClient::new(&starknet_rpc, registry_addr);
        println!("🔄 Proving (querying on-chain cache)...");
        let start = std::time::Instant::now();
        let proof_info = amd_prover.prove(&quote_bytes, &registry_client).await?;
        let elapsed = start.elapsed();

        println!("✅ Proof generated in {:.2?}", elapsed);
        println!("   Trusted prefix len: {}", proof_info.trusted_prefix_len);
        println!("   Cert chain length: {}", proof_info.cert_digests.len());

        let proof = proof_info.proof;

        // Save proof JSON
        let proof_json = proof.encode_json()?;
        std::fs::write(proof_output, &proof_json)?;
        println!("💾 Proof saved to: {}", proof_output.display());

        proof
    };

    // Convert to Starknet calldata (Garaga)
    let calldata = StarknetCalldata::from_proof(&proof)?;
    println!("📦 Calldata elements: {}", calldata.len());

    if let Some(path) = calldata_output.as_ref() {
        calldata.save_to_file(path)?;
        println!("💾 Calldata saved to: {}", path.display());
    }

    // Verify structure (skip when mock produced empty proof bytes)
    if prover != "mock" {
        AmdAttestationProver::<katana_tee_client::Sp1NetworkBackend>::verify_proof_structure(
            &proof,
        )?;
    }

    if dry_run {
        println!("🧪 Dry run: not submitting transaction.");
        return Ok(());
    }

    let account_address = account_address.ok_or_else(|| {
        anyhow::anyhow!(
            "--account-address (or STARKNET_ACCOUNT_ADDRESS) is required unless --dry-run"
        )
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
            Ok(client.fetch_attestation(None, 0).await?)
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
    use amd_tee_registry_client::{parse_processor_type, KdsClient};

    let kds = KdsClient::new();
    let mut results = serde_json::Map::new();

    for proc_str in processors.split(',') {
        let proc_str = proc_str.trim().to_lowercase();
        let proc_type = match parse_processor_type(&proc_str) {
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
                    let der_path = validate_dir.join(format!("ark-{}.der", proc_str));
                    if der_path.exists() {
                        match kds.validate_against_file(proc_type, &der_path) {
                            Ok(true) => println!("  Matches local {}", der_path.display()),
                            Ok(false) => eprintln!("  MISMATCH with local {}!", der_path.display()),
                            Err(e) => eprintln!("  Validation error: {}", e),
                        }
                    }
                }

                // Split u256 hash into low/high felt252 values as decimal numbers
                // snforge's FileParser parses JSON integers as felt252 values
                let hash = info.ark_hash.trim_start_matches("0x");
                let hash_padded = format!("{:0>64}", hash);
                let high = &hash_padded[0..32];
                let low = &hash_padded[32..64];

                // Parse hex to u128 - these will be written as decimal integers
                let high_val = u128::from_str_radix(high, 16).unwrap_or(0);
                let low_val = u128::from_str_radix(low, 16).unwrap_or(0);

                // Store as strings containing decimal numbers
                // We'll do a post-process to remove quotes for raw integers
                results.insert(
                    format!("{}_ark_hash_high", proc_str),
                    serde_json::Value::String(format!("__RAW__{}", high_val)),
                );
                results.insert(
                    format!("{}_ark_hash_low", proc_str),
                    serde_json::Value::String(format!("__RAW__{}", low_val)),
                );
            }
            Err(e) => {
                eprintln!("  Error fetching {}: {}", proc_str, e);
            }
        }
    }

    // Sort keys alphabetically for snforge compatibility
    let sorted: serde_json::Map<String, serde_json::Value> = results.into_iter().collect();
    let json_output = serde_json::to_string_pretty(&sorted)?;

    // Convert __RAW__ prefixed strings to raw JSON integers
    // This preserves u128 precision which serde_json otherwise loses
    // Simple approach: find "__RAW__<digits>" and replace with just <digits>
    let mut json_output = json_output;
    while let Some(start) = json_output.find("\"__RAW__") {
        // Find the closing quote
        if let Some(end) = json_output[start + 8..].find('"') {
            let end = start + 8 + end + 1;
            let number = &json_output[start + 8..end - 1];
            json_output = format!("{}{}{}", &json_output[..start], number, &json_output[end..]);
        } else {
            break;
        }
    }

    if let Some(output_path) = output {
        std::fs::write(&output_path, &json_output)?;
        println!("\nSaved to {}", output_path.display());
    } else {
        println!("\n{}", json_output);
    }

    Ok(())
}

fn cmd_generate_cairo_fixtures(fixture_dir: &Path, output: &Path) -> anyhow::Result<()> {
    use amd_tee_registry_client::generate_cairo_fixtures;

    println!("Generating Cairo test fixtures...");
    println!("  Fixture dir: {}", fixture_dir.display());
    println!("  Output: {}", output.display());

    generate_cairo_fixtures(fixture_dir, output)?;

    println!("Cairo fixtures generated successfully!");
    Ok(())
}

fn cmd_calldata(proof_path: &PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
    println!("📄 Loading proof from: {}", proof_path.display());

    let data = std::fs::read(proof_path)?;
    let proof = amd_sev_snp_attestation_prover::OnchainProof::decode_json(&data)?;

    println!("🔄 Generating Starknet calldata...");
    let calldata = StarknetCalldata::from_proof(&proof)?;
    println!("📦 Generated {} calldata elements", calldata.len());

    if let Some(path) = output {
        calldata.save_to_file(&path)?;
        println!("💾 Calldata saved to: {}", path.display());
    } else {
        for hex in calldata.to_hex_strings() {
            println!("{}", hex);
        }
    }

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
