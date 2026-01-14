//! Fetch TEE Attestation from Katana RPC
//!
//! This example demonstrates how to fetch a TEE attestation quote
//! from a running Katana node.
//!
//! # Usage
//!
//! ```bash
//! # Using default URL from .env or localhost:5050
//! cargo run --example fetch_attestation -p katana_tee_client
//!
//! # Using specific RPC URL
//! cargo run --example fetch_attestation -p katana_tee_client -- --rpc http://185.26.9.157:5050
//!
//! # Save to file
//! cargo run --example fetch_attestation -p katana_tee_client -- --output attestation.json
//! ```

use katana_tee_client::KatanaRpcClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("katana_tee_client=info".parse()?),
        )
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Parse arguments
    let args: Vec<String> = std::env::args().collect();
    let (rpc_url, output_file) = parse_args(&args);

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║         Katana TEE Attestation Fetcher                       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Create RPC client
    let client = KatanaRpcClient::new(&rpc_url);
    println!("🌐 RPC Endpoint: {}", client.url());
    println!();

    // Fetch attestation
    println!("🔄 Fetching TEE attestation...");
    let start = std::time::Instant::now();
    let attestation = client.fetch_attestation().await?;
    let elapsed = start.elapsed();

    println!("✅ Attestation received in {:.2?}", elapsed);
    println!();

    // Display attestation details
    println!("📋 Attestation Details:");
    println!("   Block Number: {}", attestation.block_number);
    println!("   Block Hash:   {}", attestation.block_hash);
    println!("   State Root:   {}", attestation.state_root);
    println!(
        "   Quote Size:   {} bytes",
        attestation.quote_bytes()?.len()
    );
    println!();

    // Save to file if requested
    if let Some(output) = output_file {
        let json = serde_json::to_string_pretty(&attestation)?;
        std::fs::write(&output, &json)?;
        println!("💾 Saved to: {}", output);
    } else {
        // Print quote preview
        let quote = &attestation.quote;
        let preview_len = std::cmp::min(66, quote.len()); // 0x + 32 bytes
        println!("📄 Quote Preview: {}...", &quote[..preview_len]);
    }

    Ok(())
}

fn parse_args(args: &[String]) -> (String, Option<String>) {
    let mut rpc_url =
        std::env::var("KATANA_RPC_URL").unwrap_or_else(|_| "http://localhost:5050".to_string());
    let mut output_file = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc" | "-r" => {
                if i + 1 < args.len() {
                    rpc_url = args[i + 1].clone();
                    i += 1;
                }
            }
            "--output" | "-o" => {
                if i + 1 < args.len() {
                    output_file = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--help" | "-h" => {
                println!(
                    r#"
Katana TEE Attestation Fetcher

USAGE:
    fetch_attestation [OPTIONS]

OPTIONS:
    --rpc, -r <URL>      Katana RPC endpoint (default: KATANA_RPC_URL or localhost:5050)
    --output, -o <FILE>  Save attestation to JSON file
    --help, -h           Print this help message

ENVIRONMENT VARIABLES:
    KATANA_RPC_URL       Default RPC URL

EXAMPLES:
    # Fetch from localhost
    cargo run --example fetch_attestation -p katana_tee_client

    # Fetch from specific host
    cargo run --example fetch_attestation -p katana_tee_client -- --rpc http://185.26.9.157:5050

    # Save to file
    cargo run --example fetch_attestation -p katana_tee_client -- -o attestation.json
"#
                );
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    (rpc_url, output_file)
}
