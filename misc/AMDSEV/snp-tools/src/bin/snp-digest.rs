// SPDX-License-Identifier: Apache-2.0

//! CLI tool to calculate SEV-SNP launch digest

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use sev::measurement::{
    snp::{snp_calc_launch_digest, SnpMeasurementArgs},
    vcpu_types::CpuType,
    vmsa::{GuestFeatures, VMMType},
};

#[derive(Debug, Clone, ValueEnum)]
enum CpuTypeArg {
    Epyc,
    EpycV1,
    EpycV2,
    EpycIbpb,
    EpycV3,
    EpycV4,
    EpycRome,
    EpycRomeV1,
    EpycRomeV2,
    EpycRomeV3,
    EpycMilan,
    EpycMilanV1,
    EpycMilanV2,
    EpycGenoa,
    EpycGenoaV1,
}

impl From<CpuTypeArg> for CpuType {
    fn from(value: CpuTypeArg) -> Self {
        match value {
            CpuTypeArg::Epyc => CpuType::Epyc,
            CpuTypeArg::EpycV1 => CpuType::EpycV1,
            CpuTypeArg::EpycV2 => CpuType::EpycV2,
            CpuTypeArg::EpycIbpb => CpuType::EpycIBPB,
            CpuTypeArg::EpycV3 => CpuType::EpycV3,
            CpuTypeArg::EpycV4 => CpuType::EpycV4,
            CpuTypeArg::EpycRome => CpuType::EpycRome,
            CpuTypeArg::EpycRomeV1 => CpuType::EpycRomeV1,
            CpuTypeArg::EpycRomeV2 => CpuType::EpycRomeV2,
            CpuTypeArg::EpycRomeV3 => CpuType::EpycRomeV3,
            CpuTypeArg::EpycMilan => CpuType::EpycMilan,
            CpuTypeArg::EpycMilanV1 => CpuType::EpycMilanV1,
            CpuTypeArg::EpycMilanV2 => CpuType::EpycMilanV2,
            CpuTypeArg::EpycGenoa => CpuType::EpycGenoa,
            CpuTypeArg::EpycGenoaV1 => CpuType::EpycGenoaV1,
        }
    }
}

#[derive(Debug, Clone, ValueEnum)]
enum VmmTypeArg {
    Qemu,
    Ec2,
    Krun,
}

impl From<VmmTypeArg> for VMMType {
    fn from(value: VmmTypeArg) -> Self {
        match value {
            VmmTypeArg::Qemu => VMMType::QEMU,
            VmmTypeArg::Ec2 => VMMType::EC2,
            VmmTypeArg::Krun => VMMType::KRUN,
        }
    }
}

/// Calculate SEV-SNP launch digest for attestation
#[derive(Parser, Debug)]
#[command(name = "snp-digest")]
#[command(about = "Calculate SEV-SNP launch measurement digest")]
struct Args {
    /// Path to OVMF firmware file
    #[arg(short, long)]
    ovmf: PathBuf,

    /// Number of vCPUs
    #[arg(short = 'n', long, default_value = "1")]
    vcpus: u32,

    /// CPU type
    #[arg(short, long, value_enum, default_value = "epyc-v4")]
    cpu: CpuTypeArg,

    /// VMM type
    #[arg(short, long, value_enum, default_value = "qemu")]
    vmm: VmmTypeArg,

    /// Guest features (hex value, e.g., 0x1 for SNPActive only)
    #[arg(short, long, default_value = "0x1")]
    guest_features: String,

    /// Path to kernel file (for direct kernel boot)
    #[arg(short, long)]
    kernel: Option<PathBuf>,

    /// Path to initrd file
    #[arg(short, long)]
    initrd: Option<PathBuf>,

    /// Kernel command line arguments
    #[arg(short, long)]
    append: Option<String>,

    /// Pre-calculated OVMF hash (hex string) to skip OVMF measurement
    #[arg(long)]
    ovmf_hash: Option<String>,
}

fn parse_guest_features(s: &str) -> Result<u64, String> {
    let s = s.trim().to_lowercase();
    let s = s.strip_prefix("0x").unwrap_or(&s);
    u64::from_str_radix(s, 16).map_err(|e| format!("Invalid hex value: {}", e))
}

fn main() {
    let args = Args::parse();

    let guest_features = match parse_guest_features(&args.guest_features) {
        Ok(v) => GuestFeatures(v),
        Err(e) => {
            eprintln!("Error parsing guest features: {}", e);
            std::process::exit(1);
        }
    };

    let measurement_args = SnpMeasurementArgs {
        vcpus: args.vcpus,
        vcpu_type: args.cpu.into(),
        ovmf_file: args.ovmf,
        guest_features,
        kernel_file: args.kernel,
        initrd_file: args.initrd,
        append: args.append.as_deref(),
        ovmf_hash_str: args.ovmf_hash.as_deref(),
        vmm_type: Some(args.vmm.into()),
    };

    match snp_calc_launch_digest(measurement_args) {
        Ok(digest) => {
            println!("{}", digest.get_hex_ld());
        }
        Err(e) => {
            eprintln!("Error calculating launch digest: {:?}", e);
            std::process::exit(1);
        }
    }
}
