// SPDX-License-Identifier: Apache-2.0

//! CLI tool to decode SEV-SNP attestation reports

use std::io::{self, Read};
use std::path::PathBuf;

use clap::Parser;
use sev::firmware::guest::AttestationReport;
use sev::parser::ByteParser;
use tabled::settings::style::Style;
use tabled::settings::Padding;
use tabled::{Table, Tabled};

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

    /// Output raw format (original verbose output)
    #[arg(short, long, default_value = "false")]
    raw: bool,
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

#[derive(Tabled)]
struct ReportRow {
    #[tabled(rename = "Field")]
    field: &'static str,
    #[tabled(rename = "Value")]
    value: String,
}

fn version_string(version: u32) -> String {
    match version {
        2 => "2".to_string(),
        3 => "3 (Milan)".to_string(),
        5 => "5 (Genoa/Turin)".to_string(),
        v => format!("{}", v),
    }
}

fn policy_string(policy: u64) -> String {
    let mut flags = Vec::new();

    if (policy >> 16) & 1 == 1 {
        flags.push("SMT allowed");
    }
    if (policy >> 19) & 1 == 1 {
        flags.push("Debug");
    }
    if (policy >> 18) & 1 == 1 {
        flags.push("Migrate MA");
    }
    if (policy >> 20) & 1 == 1 {
        flags.push("Single socket");
    }

    if flags.is_empty() {
        format!("{:#x}", policy)
    } else {
        format!("{:#x} ({})", policy, flags.join(", "))
    }
}

fn signing_key_string(key_info: u32) -> &'static str {
    match key_info & 0b111 {
        0 => "VCEK",
        1 => "VLEK",
        _ => "Unknown",
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn truncate_hex(hex: &str, max_len: usize) -> String {
    if hex.len() > max_len {
        format!("{}...", &hex[..max_len])
    } else {
        hex.to_string()
    }
}

fn print_table_report(report: &AttestationReport) {
    let measurement = bytes_to_hex(&report.measurement);
    let report_data = bytes_to_hex(&report.report_data);
    let report_id = bytes_to_hex(&report.report_id);
    let host_data = bytes_to_hex(&report.host_data);
    let id_key_digest = bytes_to_hex(&report.id_key_digest);
    let author_key_digest = bytes_to_hex(&report.author_key_digest);
    let chip_id = bytes_to_hex(&report.chip_id);

    let rows = vec![
        ReportRow {
            field: "Version",
            value: version_string(report.version),
        },
        ReportRow {
            field: "Measurement",
            value: measurement,
        },
        ReportRow {
            field: "Guest SVN",
            value: format!("{}", report.guest_svn),
        },
        ReportRow {
            field: "VMPL",
            value: format!("{}", report.vmpl),
        },
        ReportRow {
            field: "Policy",
            value: policy_string(report.policy.0),
        },
        ReportRow {
            field: "Report Data",
            value: truncate_hex(&report_data, 64),
        },
        ReportRow {
            field: "Report ID",
            value: report_id,
        },
        ReportRow {
            field: "Host Data",
            value: if host_data.chars().all(|c| c == '0') {
                "(empty)".to_string()
            } else {
                truncate_hex(&host_data, 64)
            },
        },
        ReportRow {
            field: "Signing Key",
            value: signing_key_string(report.key_info.0).to_string(),
        },
    ];

    println!("Attestation Report");
    let table = Table::new(rows)
        .with(Style::rounded())
        .with(Padding::new(1, 1, 0, 0))
        .to_string();
    println!("{}", table);

    // TCB Info
    println!();
    println!("TCB Version");
    let tcb_rows = vec![
        ReportRow {
            field: "Boot Loader",
            value: format!("{}", report.reported_tcb.bootloader),
        },
        ReportRow {
            field: "TEE",
            value: format!("{}", report.reported_tcb.tee),
        },
        ReportRow {
            field: "SNP",
            value: format!("{}", report.reported_tcb.snp),
        },
        ReportRow {
            field: "Microcode",
            value: format!("{}", report.reported_tcb.microcode),
        },
    ];
    let tcb_table = Table::new(tcb_rows)
        .with(Style::rounded())
        .with(Padding::new(1, 1, 0, 0))
        .to_string();
    println!("{}", tcb_table);

    // Platform Info
    println!();
    println!("Platform Info");
    let plat_info = report.plat_info.0;
    let platform_rows = vec![
        ReportRow {
            field: "SMT Enabled",
            value: format!("{}", plat_info & 1 == 1),
        },
        ReportRow {
            field: "TSME Enabled",
            value: format!("{}", (plat_info >> 1) & 1 == 1),
        },
        ReportRow {
            field: "ECC Enabled",
            value: format!("{}", (plat_info >> 2) & 1 == 1),
        },
    ];
    let platform_table = Table::new(platform_rows)
        .with(Style::rounded())
        .with(Padding::new(1, 1, 0, 0))
        .to_string();
    println!("{}", platform_table);

    // Additional Info
    println!();
    println!("Additional Info");

    let family_str = report
        .cpuid_fam_id
        .map(|f| format!("{} (0x{:x})", f, f))
        .unwrap_or_else(|| "N/A".to_string());

    let model_str = report
        .cpuid_mod_id
        .map(|m| format!("{} (0x{:x})", m, m))
        .unwrap_or_else(|| "N/A".to_string());

    let stepping_str = report
        .cpuid_step
        .map(|s| format!("{}", s))
        .unwrap_or_else(|| "N/A".to_string());

    let additional_rows = vec![
        ReportRow {
            field: "CPUID Family",
            value: family_str,
        },
        ReportRow {
            field: "CPUID Model",
            value: model_str,
        },
        ReportRow {
            field: "CPUID Stepping",
            value: stepping_str,
        },
        ReportRow {
            field: "ID Key Digest",
            value: if id_key_digest.chars().all(|c| c == '0') {
                "(empty)".to_string()
            } else {
                truncate_hex(&id_key_digest, 32)
            },
        },
        ReportRow {
            field: "Author Key Digest",
            value: if author_key_digest.chars().all(|c| c == '0') {
                "(empty)".to_string()
            } else {
                truncate_hex(&author_key_digest, 32)
            },
        },
        ReportRow {
            field: "Chip ID",
            value: truncate_hex(&chip_id, 32),
        },
    ];
    let additional_table = Table::new(additional_rows)
        .with(Style::rounded())
        .with(Padding::new(1, 1, 0, 0))
        .to_string();
    println!("{}", additional_table);
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

    if args.raw {
        // Use the original Display implementation
        println!("{}", report);
    } else {
        // Use the table format
        print_table_report(&report);
    }
}
