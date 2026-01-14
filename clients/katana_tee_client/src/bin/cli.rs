//! Katana TEE CLI
//!
//! A command-line interface for interacting with Katana TEE attestations
//! and generating SP1 proofs.

use clap::{Parser, Subcommand};
use katana_tee_client::{
    generate_sp1_proof, prover::verify_proof_structure, KatanaRpcClient, TeeQuoteResponse,
};
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

        /// Output file for the proof
        #[arg(short, long, default_value = "proof_output.json")]
        output: PathBuf,
    },

    /// Display information about a proof file
    Info {
        /// Path to the proof JSON file
        #[arg(default_value = "proof_output.json")]
        file: PathBuf,
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
            output,
        } => cmd_prove(&rpc, json, &prover, &output).await,
        Commands::Info { file } => cmd_info(&file),
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
    let proof = generate_sp1_proof(attestation).await?;
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
    output: &PathBuf,
) -> anyhow::Result<()> {
    std::env::set_var("SP1_PROVER", prover);

    // Validate network mode has key
    if prover == "network" {
        let has_key = std::env::var("NETWORK_PRIVATE_KEY").is_ok()
            || std::env::var("SP1_PRIVATE_KEY").is_ok();
        if !has_key {
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
    let proof = generate_sp1_proof(attestation).await?;
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
