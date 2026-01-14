//! Generate SP1 Proof from Katana TEE Attestation
//!
//! This example demonstrates how to generate an SP1 Groth16 proof
//! from a Katana TEE attestation quote.
//!
//! # Usage
//!
//! ```bash
//! # Using JSON file (default: example_response.json)
//! cargo run --example generate_proof
//!
//! # Using a specific JSON file
//! cargo run --example generate_proof -- --json path/to/response.json
//!
//! # Using Katana RPC (future support)
//! cargo run --example generate_proof -- --rpc http://localhost:5050
//! ```
//!
//! # Environment Variables
//!
//! - `SP1_PROVER`: Set to "mock" for local testing, "network" for real proving
//! - `NETWORK_PRIVATE_KEY`: Required for network proving (SP1 Prover Network)
//! - `SKIP_TIME_VALIDITY_CHECK`: Set to "true" to skip time checks (for old attestations)

use katana_tee_client::{
    generate_sp1_proof, prover::verify_proof_structure, KatanaRpcClient, TeeQuoteResponse,
};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("katana_tee_client=debug".parse()?)
                .add_directive("amd_sev_snp_attestation_prover=info".parse()?),
        )
        .init();

    // Load environment variables from .env file if present
    dotenvy::dotenv().ok();

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let (mode, path_or_url) = parse_args(&args);

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║         Katana TEE SP1 Proof Generator                       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Load the TEE quote response
    let response = match mode.as_str() {
        "json" => {
            let path = PathBuf::from(&path_or_url);
            println!("📄 Loading attestation from: {}", path.display());
            TeeQuoteResponse::from_json_file(&path)?
        }
        "rpc" => {
            println!("🌐 Fetching attestation from RPC: {}", path_or_url);
            let client = KatanaRpcClient::new(&path_or_url);
            client.fetch_attestation().await?
        }
        _ => unreachable!(),
    };

    println!();
    println!("📋 Attestation Details:");
    println!("   Block Number: {}", response.block_number);
    println!("   Block Hash:   {}", response.block_hash);
    println!("   State Root:   {}", response.state_root);
    println!("   Quote Size:   {} bytes", response.quote_bytes()?.len());
    println!();

    // Check prover mode
    let prover_mode = std::env::var("SP1_PROVER").unwrap_or_else(|_| "mock".to_string());
    println!("⚙️  Prover Configuration:");
    println!("   Mode: {}", prover_mode);
    if prover_mode == "network" {
        // NETWORK_PRIVATE_KEY is the standard env var used by SP1 SDK for network proving
        let has_key = std::env::var("NETWORK_PRIVATE_KEY").is_ok()
            || std::env::var("SP1_PRIVATE_KEY").is_ok();
        println!(
            "   Private Key: {}",
            if has_key { "✓ Set" } else { "✗ Missing" }
        );
        if !has_key {
            anyhow::bail!("NETWORK_PRIVATE_KEY environment variable required for network mode");
        }
    }
    println!();

    println!("🔄 Generating SP1 proof...");
    println!("   (This may take several minutes for Groth16 proving)");
    println!();

    let start = std::time::Instant::now();
    let proof = generate_sp1_proof(response).await?;
    let elapsed = start.elapsed();

    println!("✅ Proof generated successfully!");
    println!();
    println!("📊 Proof Details:");
    println!("   ZK Type:      {:?}", proof.zktype);
    println!("   ZKVM Version: {}", proof.zkvm_version);
    println!("   Verifier ID:  {}", proof.program_id.verifier_id);
    println!("   Proof Size:   {} bytes", proof.onchain_proof.len());
    println!("   Time Elapsed: {:.2?}", elapsed);
    println!();

    // Verify proof structure (skip in mock mode - mock doesn't generate actual proof bytes)
    if prover_mode == "mock" {
        if proof.onchain_proof.is_empty() {
            println!("ℹ️  Mock mode: Proof bytes are empty (expected - use 'network' mode for real proofs)");
        } else {
            verify_proof_structure(&proof)?;
            println!("✅ Proof structure verified!");
        }
    } else {
        verify_proof_structure(&proof)?;
        println!("✅ Proof structure verified!");
    }
    println!();

    // Save proof to file
    let output_path = "proof_output.json";
    let proof_json = proof.encode_json()?;
    std::fs::write(output_path, &proof_json)?;
    println!("💾 Proof saved to: {}", output_path);

    Ok(())
}

fn parse_args(args: &[String]) -> (String, String) {
    let mut mode = "json".to_string();
    let mut path_or_url = "clients/katana_tee_client/example_response.json".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                mode = "json".to_string();
                if i + 1 < args.len() {
                    path_or_url = args[i + 1].clone();
                    i += 1;
                }
            }
            "--rpc" => {
                mode = "rpc".to_string();
                if i + 1 < args.len() {
                    path_or_url = args[i + 1].clone();
                    i += 1;
                } else {
                    path_or_url = std::env::var("KATANA_RPC_URL")
                        .unwrap_or_else(|_| "http://localhost:5050".to_string());
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    (mode, path_or_url)
}

fn print_help() {
    println!(
        r#"
Katana TEE SP1 Proof Generator

USAGE:
    generate_proof [OPTIONS]

OPTIONS:
    --json <PATH>    Load attestation from JSON file (default: example_response.json)
    --rpc <URL>      Fetch attestation from Katana RPC endpoint
    --help, -h       Print this help message

ENVIRONMENT VARIABLES:
    SP1_PROVER                 Set to "mock" for local testing, "network" for real proving
    NETWORK_PRIVATE_KEY        Required for network proving (SP1 Prover Network)
    SKIP_TIME_VALIDITY_CHECK   Set to "true" to skip time validity checks
    KATANA_RPC_URL             Default RPC URL when using --rpc without a URL

EXAMPLES:
    # Generate proof from JSON file with mock prover
    SP1_PROVER=mock cargo run --example generate_proof

    # Generate proof with network prover (using .env file)
    SP1_PROVER=network cargo run --example generate_proof

    # Or set NETWORK_PRIVATE_KEY directly
    SP1_PROVER=network NETWORK_PRIVATE_KEY=your_key cargo run --example generate_proof

    # Use a different JSON file
    cargo run --example generate_proof -- --json my_attestation.json
"#
    );
}
