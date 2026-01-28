// SPDX-License-Identifier: Apache-2.0

//! CLI tool to decode SEV-SNP attestation reports

use std::io::{self, Read};
use std::path::PathBuf;

use clap::Parser;
use sev::firmware::guest::AttestationReport;
use sev::parser::ByteParser;

/// Decode SEV-SNP attestation reports
#[derive(Parser, Debug)]
#[command(name = "snp-report")]
#[command(about = "Decode and display SEV-SNP attestation reports")]
struct Args {
    /// Hex-encoded attestation report (with or without 0x prefix)
    #[arg(short = 'x', long)]
    hex: Option<String>,

    /// Path to file containing the attestation report (binary or hex)
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// Input is binary (not hex-encoded)
    #[arg(short, long, default_value = "false")]
    binary: bool,
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let hex = hex.trim();
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    let hex = hex.strip_prefix("0X").unwrap_or(hex);

    // Remove any whitespace or newlines
    let hex: String = hex.chars().filter(|c| !c.is_whitespace()).collect();

    if hex.len() % 2 != 0 {
        return Err("Hex string must have even length".to_string());
    }

    hex::decode(&hex).map_err(|e| format!("Invalid hex: {}", e))
}

fn read_input(args: &Args) -> Result<Vec<u8>, String> {
    if let Some(hex_str) = &args.hex {
        return decode_hex(hex_str);
    }

    if let Some(file_path) = &args.file {
        let data = std::fs::read(file_path).map_err(|e| format!("Failed to read file: {}", e))?;

        if args.binary {
            return Ok(data);
        }

        // Try to decode as hex first
        let hex_str = String::from_utf8(data.clone());
        if let Ok(hex) = hex_str {
            if let Ok(decoded) = decode_hex(&hex) {
                return Ok(decoded);
            }
        }

        // Fall back to binary
        Ok(data)
    } else {
        // Read from stdin
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| format!("Failed to read stdin: {}", e))?;

        if args.binary {
            Ok(buffer.into_bytes())
        } else {
            decode_hex(&buffer)
        }
    }
}

fn main() {
    let args = Args::parse();

    let data = match read_input(&args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading input: {}", e);
            std::process::exit(1);
        }
    };

    if data.len() < 1184 {
        eprintln!(
            "Error: Input too short. Expected at least 1184 bytes, got {}",
            data.len()
        );
        std::process::exit(1);
    }

    let report = match AttestationReport::from_bytes(&data[..1184]) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error decoding attestation report: {:?}", e);
            std::process::exit(1);
        }
    };

    // Use the Display implementation
    println!("{}", report);
}
