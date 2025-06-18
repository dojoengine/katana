use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use goose::prelude::*;
use std::time::Duration;
use tracing::{info, warn};

mod client;
mod scenarios;
mod transactions;
mod utils;

use client::StarknetClient;
use scenarios::{burst_load, constant_load, ramp_up_load};

#[derive(Parser)]
#[command(name = "katana-load-test")]
#[command(about = "Load testing tool for Katana sequencer")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[command(flatten)]
    pub global: GlobalArgs,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run constant load test
    Constant(ConstantLoadArgs),
    /// Run ramp-up load test
    RampUp(RampUpArgs),
    /// Run burst load test
    Burst(BurstArgs),
    /// Run custom scenario
    Custom(CustomArgs),
}

#[derive(Args)]
pub struct GlobalArgs {
    /// Katana RPC URL
    #[arg(long, default_value = "http://localhost:5050")]
    pub rpc_url: String,

    /// Private key for transaction signing (hex)
    #[arg(long, env = "KATANA_PRIVATE_KEY")]
    pub private_key: Option<String>,

    /// Account address (hex)
    #[arg(long, env = "KATANA_ACCOUNT")]
    pub account_address: Option<String>,

    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,

    /// Output file for metrics (JSON)
    #[arg(long)]
    pub output: Option<String>,

    /// Generate HTML report
    #[arg(long)]
    pub html_report: Option<String>,

    /// Enable real-time dashboard (WebSocket on port 5117)
    #[arg(long)]
    pub dashboard: bool,

    /// Disable auto-start (wait for controller command)
    #[arg(long)]
    pub no_autostart: bool,
}

#[derive(Args)]
pub struct ConstantLoadArgs {
    /// Transactions per second
    #[arg(long, default_value = "10")]
    pub tps: u64,

    /// Test duration in seconds
    #[arg(long, default_value = "60")]
    pub duration: u64,

    /// Number of concurrent users
    #[arg(long, default_value = "10")]
    pub users: usize,
}

#[derive(Args)]
pub struct RampUpArgs {
    /// Starting TPS
    #[arg(long, default_value = "1")]
    pub start_tps: u64,

    /// Maximum TPS
    #[arg(long, default_value = "100")]
    pub max_tps: u64,

    /// Ramp-up duration in seconds
    #[arg(long, default_value = "300")]
    pub ramp_duration: u64,

    /// Hold duration at max TPS
    #[arg(long, default_value = "60")]
    pub hold_duration: u64,
}

#[derive(Args)]
pub struct BurstArgs {
    /// Normal TPS (baseline)
    #[arg(long, default_value = "10")]
    pub baseline_tps: u64,

    /// Burst TPS
    #[arg(long, default_value = "100")]
    pub burst_tps: u64,

    /// Burst duration in seconds
    #[arg(long, default_value = "30")]
    pub burst_duration: u64,

    /// Total test duration in seconds
    #[arg(long, default_value = "300")]
    pub total_duration: u64,
}

#[derive(Args)]
pub struct CustomArgs {
    /// Number of users
    #[arg(long, default_value = "50")]
    pub users: usize,

    /// Spawn rate (users per second)
    #[arg(long, default_value = "2")]
    pub spawn_rate: f64,

    /// Test duration in seconds
    #[arg(long, default_value = "300")]
    pub duration: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    let level = if cli.global.debug {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .init();

    info!("Starting Katana load test");
    info!("RPC URL: {}", cli.global.rpc_url);

    // Validate connection to Katana
    let client = StarknetClient::new(&cli.global.rpc_url)?;
    client.validate_connection().await?;

    // Build base Goose attack with enhanced metrics
    let mut base_attack = GooseAttack::initialize()?;

    // Configure enhanced reporting and monitoring
    if let Some(html_file) = &cli.global.html_report {
        base_attack = base_attack.set_default(GooseDefault::ReportFile, html_file.as_str())?;
    }

    if cli.global.no_autostart {
        base_attack = base_attack.set_default(GooseDefault::NoAutoStart, true)?;
        info!("Load test ready - use telnet (port 5116) or WebSocket (port 5117) to control");
    }

    if cli.global.dashboard {
        info!("Real-time dashboard available at: ws://localhost:5117");
    }

    // Add detailed metrics configuration
    base_attack = base_attack
        .set_default(GooseDefault::NoMetrics, false)?
        .set_default(GooseDefault::StatusCodes, true)?;

    // Build specific attack based on command
    let attack = match cli.command {
        Commands::Constant(args) => {
            info!(
                "Running constant load test: {} TPS for {} seconds with {} users",
                args.tps, args.duration, args.users
            );
            
            constant_load(
                base_attack,
                cli.global,
                args.users,
                Duration::from_secs(args.duration),
                args.tps,
            ).await?
        }
        Commands::RampUp(args) => {
            info!(
                "Running ramp-up test: {} -> {} TPS over {} seconds, hold for {} seconds",
                args.start_tps, args.max_tps, args.ramp_duration, args.hold_duration
            );
            
            ramp_up_load(base_attack, cli.global, args).await?
        }
        Commands::Burst(args) => {
            info!(
                "Running burst test: {} TPS baseline with {} TPS bursts for {} seconds",
                args.baseline_tps, args.burst_tps, args.burst_duration
            );
            
            burst_load(base_attack, cli.global, args).await?
        }
        Commands::Custom(args) => {
            info!(
                "Running custom test: {} users, spawn rate {}/s for {} seconds",
                args.users, args.spawn_rate, args.duration
            );
            
            base_attack
                .register_scenario(
                    scenario!("KatanaLoadTest")
                        .register_transaction(
                            transaction!(scenarios::send_transaction)
                                .set_name("send_erc20_transfer")
                                .set_weight(10)?
                        )
                        .register_transaction(
                            transaction!(scenarios::check_transaction_status)
                                .set_name("check_tx_status")
                                .set_weight(2)?
                        )
                )
                .set_default(
                    GooseDefault::Host,
                    cli.global.rpc_url.as_str(),
                )?
                .set_default(GooseDefault::Users, args.users)?
                .set_default(GooseDefault::HatchRate, args.spawn_rate)?
                .set_default(GooseDefault::RunTime, args.duration)?
        }
    };

    // Execute the load test
    let goose_metrics = attack.execute().await?;

    // Print summary
    print_summary(&goose_metrics);

    // Save results if output file specified
    if let Some(output_file) = cli.global.output {
        save_results(&goose_metrics, &output_file)?;
    }

    Ok(())
}

fn print_summary(metrics: &GooseMetrics) {
    info!("=== Load Test Summary ===");
    info!("Total requests: {}", metrics.requests.len());
    info!("Total users: {}", metrics.users);
    
    if let Some(final_metrics) = metrics.requests.get("Aggregated") {
        info!("Success rate: {:.2}%", 
            (final_metrics.success_count as f64 / final_metrics.raw_data.counter as f64) * 100.0
        );
        info!("Average response time: {:.2}ms", final_metrics.raw_data.mean);
        info!("95th percentile: {:.2}ms", final_metrics.raw_data.percentile_95);
        info!("99th percentile: {:.2}ms", final_metrics.raw_data.percentile_99);
        info!("Requests per second: {:.2}", final_metrics.requests_per_second);
    }

    if !metrics.errors.is_empty() {
        warn!("Errors encountered:");
        for (error, count) in &metrics.errors {
            warn!("  {}: {} occurrences", error, count);
        }
    }
}

fn save_results(metrics: &GooseMetrics, output_file: &str) -> Result<()> {
    let json = serde_json::to_string_pretty(metrics)?;
    std::fs::write(output_file, json)?;
    info!("Results saved to: {}", output_file);
    Ok(())
}
