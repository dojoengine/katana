// SPDX-License-Identifier: Apache-2.0

//! CLI tool to extract OVMF SEV metadata sections

use std::path::PathBuf;

use clap::Parser;
use sev::measurement::ovmf::{SectionType, OVMF};

/// Extract OVMF SEV metadata sections
#[derive(Parser, Debug)]
#[command(name = "ovmf-metadata")]
#[command(about = "Extract and display OVMF SEV metadata sections")]
struct Args {
    /// Path to OVMF firmware file
    #[arg(short, long)]
    ovmf: PathBuf,
}

fn section_type_name(section_type: SectionType) -> &'static str {
    match section_type {
        SectionType::SnpSecMemory => "SNP_SEC_MEM",
        SectionType::SnpSecrets => "SNP_SECRETS",
        SectionType::Cpuid => "CPUID",
        SectionType::SvsmCaa => "SVSM_CAA",
        SectionType::SnpKernelHashes => "SNP_KERNEL_HASHES",
    }
}

fn main() {
    let args = Args::parse();

    let ovmf = match OVMF::new(args.ovmf) {
        Ok(ovmf) => ovmf,
        Err(e) => {
            eprintln!("Error parsing OVMF file: {:?}", e);
            std::process::exit(1);
        }
    };

    let metadata_items = ovmf.metadata_items();

    if metadata_items.is_empty() {
        println!("No metadata sections found.");
        return;
    }

    println!("OVMF GPA: {:#x}", ovmf.gpa());
    println!("OVMF size: {} bytes", ovmf.data().len());
    println!();
    println!("Metadata sections ({} items):", metadata_items.len());
    println!("{:-<60}", "");
    println!("{:<20} {:>12} {:>12}", "TYPE", "GPA", "SIZE");
    println!("{:-<60}", "");

    for item in metadata_items {
        println!(
            "{:<20} {:#12x} {:#12x}",
            section_type_name(item.section_type),
            item.gpa,
            item.size
        );
    }

    println!("{:-<60}", "");

    // Print additional info about supported features
    println!();
    if ovmf.is_sev_hashes_table_supported() {
        if let Ok(gpa) = ovmf.sev_hashes_table_gpa() {
            println!("SEV hashes table supported at GPA: {:#x}", gpa);
        }
    } else {
        println!("SEV hashes table: not supported");
    }

    if let Ok(eip) = ovmf.sev_es_reset_eip() {
        println!("SEV-ES reset EIP: {:#x}", eip);
    }
}
