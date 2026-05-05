//! AMD KDS (Key Distribution Service) client CLI.
//!
//! Fetch root certificate hashes from <https://kdsintf.amd.com> and optionally
//! validate them against a local DER file.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use katana_tee::amd::kds::{parse_processor_type, KdsClient, RootCertInfo};

#[derive(Parser)]
#[command(name = "kds-client", about = "AMD KDS (Key Distribution Service) client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch root certificate hash(es) from AMD KDS.
    Fetch {
        /// Comma-separated processor models (milan, genoa, bergamo, siena).
        #[arg(long, value_delimiter = ',', default_values_t = default_processors())]
        processors: Vec<String>,

        /// Write results as JSON to this path. If omitted, prints labeled output to stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Validate a local DER file's hash against AMD KDS.
    Validate {
        /// Processor model (milan, genoa, bergamo, siena).
        #[arg(long)]
        processor: String,

        /// Path to the local .der file.
        #[arg(long)]
        der: PathBuf,
    },
}

fn default_processors() -> Vec<String> {
    vec!["milan".into(), "genoa".into(), "bergamo".into(), "siena".into()]
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = KdsClient::new();

    match cli.command {
        Command::Fetch { processors, output } => {
            let parsed = processors
                .iter()
                .map(|name| {
                    parse_processor_type(name)
                        .ok_or_else(|| anyhow!("unknown processor model: '{name}'"))
                })
                .collect::<Result<Vec<_>>>()?;

            let results: HashMap<String, RootCertInfo> = client
                .fetch_root_certs(&parsed)
                .with_context(|| "failed to fetch root certs from AMD KDS")?;

            if let Some(path) = output {
                let json = serde_json::to_string_pretty(&results)?;
                std::fs::write(&path, json)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                println!("wrote {} entries to {}", results.len(), path.display());
            } else {
                let mut entries: Vec<_> = results.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                for (proc_name, info) in entries {
                    println!("processor: {proc_name}");
                    println!("  ark_hash: {}", info.ark_hash);
                    println!("  source:   {}", info.source);
                }
            }
        }

        Command::Validate { processor, der } => {
            let processor = parse_processor_type(&processor)
                .ok_or_else(|| anyhow!("unknown processor model: '{processor}'"))?;

            let matches = client
                .validate_against_file(processor, &der)
                .with_context(|| format!("failed to validate {}", der.display()))?;

            if matches {
                println!("match: local DER hash equals KDS-fetched ARK hash");
            } else {
                println!("mismatch: local DER hash does NOT equal KDS-fetched ARK hash");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
