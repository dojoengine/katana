//! Execute SP1 Proof (Mock Mode)
//!
//! This example demonstrates how to fetch a TEE attestation and execute
//! the SP1 program in mock mode. This is fast and useful for testing
//! the proof inputs without actual proving.
//!
//! # Usage
//!
//! ```bash
//! # Execute with mock prover (fast, no real proof)
//! cargo run --example execute_proof -p katana_tee_client --release
//!
//! # Using specific RPC URL
//! cargo run --example execute_proof -p katana_tee_client --release -- --rpc http://185.26.9.157:5050
//!
//! # Using JSON file instead of RPC
//! cargo run --example execute_proof -p katana_tee_client --release -- --json attestation.json
//! ```

use katana_tee_client::{generate_sp1_proof, KatanaRpcClient, TeeQuoteResponse};
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

    // Load environment variables
    dotenvy::dotenv().ok();

    // Force mock mode for execution
    std::env::set_var("SP1_PROVER", "mock");

    // Parse arguments
    let args: Vec<String> = std::env::args().collect();
    let source = parse_args(&args);

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║         Katana TEE SP1 Executor (Mock Mode)                  ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Get attestation from source
    let attestation = match source {
        Source::Rpc(url) => {
            println!("🌐 Fetching attestation from RPC: {}", url);
            let client = KatanaRpcClient::new(&url);
            client.fetch_attestation().await?
        }
        Source::File(path) => {
            println!("📄 Loading attestation from: {}", path.display());
            TeeQuoteResponse::from_json_file(&path)?
        }
    };

    println!();
    println!("📋 Attestation Details:");
    println!("   Block Number: {}", attestation.block_number);
    println!("   Block Hash:   {}", attestation.block_hash);
    println!("   State Root:   {}", attestation.state_root);
    println!(
        "   Quote Size:   {} bytes",
        attestation.quote_bytes()?.len()
    );
    println!();

    println!("⚙️  Mode: Mock (execute only, no real proof)");
    println!();

    println!("🔄 Executing SP1 program...");
    let start = std::time::Instant::now();
    let proof = generate_sp1_proof(attestation).await?;
    let elapsed = start.elapsed();

    println!("✅ Execution completed in {:.2?}", elapsed);
    println!();
    println!("📊 Result:");
    println!("   ZK Type:      {:?}", proof.zktype);
    println!("   ZKVM Version: {}", proof.zkvm_version);
    println!("   Verifier ID:  {}", proof.program_id.verifier_id);
    println!();
    println!("ℹ️  Note: Mock mode produces empty proof bytes.");
    println!("   Use 'generate_proof' with SP1_PROVER=network for real proofs.");

    Ok(())
}

enum Source {
    Rpc(String),
    File(PathBuf),
}

fn parse_args(args: &[String]) -> Source {
    let default_rpc =
        std::env::var("KATANA_RPC_URL").unwrap_or_else(|_| "http://localhost:5050".to_string());

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" | "-r" => {
                if i + 1 < args.len() {
                    return Source::Rpc(args[i + 1].clone());
                }
                return Source::Rpc(default_rpc);
            }
            "--json" | "-j" => {
                if i + 1 < args.len() {
                    return Source::File(PathBuf::from(&args[i + 1]));
                }
            }
            "--help" | "-h" => {
                println!(
                    r#"
Katana TEE SP1 Executor (Mock Mode)

Executes the SP1 program without generating a real proof.
Useful for testing and validating attestation inputs quickly.

USAGE:
    execute_proof [OPTIONS]

OPTIONS:
    --rpc, -r <URL>      Fetch attestation from Katana RPC
    --json, -j <FILE>    Load attestation from JSON file
    --help, -h           Print this help message

ENVIRONMENT VARIABLES:
    KATANA_RPC_URL               Default RPC URL
    SKIP_TIME_VALIDITY_CHECK     Set to "true" to skip time checks

EXAMPLES:
    # Execute with RPC attestation
    cargo run --example execute_proof -p katana_tee_client --release -- --rpc http://185.26.9.157:5050

    # Execute with JSON file
    cargo run --example execute_proof -p katana_tee_client --release -- --json attestation.json
"#
                );
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    // Default to RPC
    Source::Rpc(default_rpc)
}
