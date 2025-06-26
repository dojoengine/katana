//! Standalone command-line tool for database verification.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Arg, Command};
use katana_provider::providers::db::DbProvider;
use katana_db::mdbx::{DbEnv, DbEnvKind};
use katana_verifier::DatabaseVerifier;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

fn main() -> Result<()> {
    // Parse command line arguments
    let matches = Command::new("katana-verifier")
        .version("0.1.0")
        .author("Katana Team")
        .about("Katana database integrity verification tool")
        .arg(
            Arg::new("database")
                .short('d')
                .long("database")
                .value_name("PATH")
                .help("Path to the database directory")
                .required(true),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_name("FILE")
                .help("Output file for verification report (JSON format)")
                .required(false),
        )
        .arg(
            Arg::new("range")
                .short('r')
                .long("range")
                .value_name("START:END")
                .help("Verify only a specific block range (e.g., 100:200)")
                .required(false),
        )
        .arg(
            Arg::new("sample")
                .short('s')
                .long("sample")
                .value_name("RATE")
                .help("Sample every Nth block instead of verifying all blocks")
                .required(false),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Enable verbose logging")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    // Initialize logging
    let log_level = if matches.get_flag("verbose") { Level::DEBUG } else { Level::INFO };
    let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    // Get database path
    let db_path = matches
        .get_one::<String>("database")
        .expect("database argument is required");
    let db_path = PathBuf::from(db_path);

    info!("Opening database at: {}", db_path.display());

    // Open database
    let db_env = DbEnv::open(&db_path, DbEnvKind::RO).context("Failed to open database")?;
    let database = DbProvider::new(db_env);

    // Create verifier
    let verifier = DatabaseVerifier::new();

    // Run verification based on arguments
    let report = if let Some(range_str) = matches.get_one::<String>("range") {
        // Parse range
        let parts: Vec<&str> = range_str.split(':').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid range format. Use START:END (e.g., 100:200)");
        }

        let start: u64 = parts[0].parse().context("Invalid start block number")?;
        let end: u64 = parts[1].parse().context("Invalid end block number")?;

        info!("Verifying block range {} to {}", start, end);
        verifier.verify_block_range(&database, start, end)?
    } else if let Some(sample_str) = matches.get_one::<String>("sample") {
        let sample_rate: u64 = sample_str.parse().context("Invalid sample rate")?;
        
        if sample_rate == 0 {
            anyhow::bail!("Sample rate must be greater than 0");
        }

        info!("Verifying with sample rate 1/{}", sample_rate);
        verifier.verify_sample(&database, sample_rate)?
    } else {
        info!("Verifying entire database");
        verifier.verify_database(&database)?
    };

    // Print report to console
    println!("{}", report);

    // Write report to file if specified
    if let Some(output_file) = matches.get_one::<String>("output") {
        let json_report = report.to_json().context("Failed to serialize report to JSON")?;
        std::fs::write(output_file, json_report)
            .with_context(|| format!("Failed to write report to {}", output_file))?;
        info!("Verification report written to: {}", output_file);
    }

    // Exit with appropriate code
    if report.is_success() {
        info!("Database verification completed successfully!");
        Ok(())
    } else {
        info!("Database verification found {} failures", report.failed_count());
        std::process::exit(1);
    }
}
