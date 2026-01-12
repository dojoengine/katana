//! AMD SEV-SNP Attestation Report Parsing
//!
//! This module provides types and parsing for AMD SEV-SNP attestation reports.
//! The structures match those defined in the Cairo contracts for consistency.
//!
//! Based on AMD SEV-SNP ABI Specification (Table 23)
//! Total report size: 1184 bytes (296 u32 words)

use crate::types::{ChipId, ProcessorType, TcbVersion};
use crate::Error;

/// Total size of attestation report in bytes
pub const ATTESTATION_REPORT_SIZE: usize = 1184;

/// Signature size in bytes (512 bytes for ECDSA P-384)
pub const SIGNATURE_SIZE: usize = 512;

// ============================================================================
// Byte offsets for attestation report fields
// ============================================================================

const OFF_VERSION: usize = 0x00;
const OFF_GUEST_SVN: usize = 0x04;
const OFF_POLICY: usize = 0x08;
const OFF_FAMILY_ID: usize = 0x10;
const OFF_IMAGE_ID: usize = 0x20;
const OFF_VMPL: usize = 0x30;
const OFF_SIG_ALGO: usize = 0x34;
const OFF_CURRENT_TCB: usize = 0x38;
const OFF_PLAT_INFO: usize = 0x40;
const OFF_AUTHOR_KEY_EN: usize = 0x48;
#[allow(dead_code)]
const OFF_RESERVED0: usize = 0x4C;
const OFF_REPORT_DATA: usize = 0x50;
const OFF_MEASUREMENT: usize = 0x90;
const OFF_HOST_DATA: usize = 0xC0;
const OFF_ID_KEY_DIGEST: usize = 0xE0;
const OFF_AUTHOR_KEY_DIGEST: usize = 0x110;
const OFF_REPORT_ID: usize = 0x140;
const OFF_REPORT_ID_MA: usize = 0x160;
const OFF_REPORTED_TCB: usize = 0x180;
#[allow(dead_code)]
const OFF_RESERVED1: usize = 0x188;
const OFF_CHIP_ID: usize = 0x1A0;
const OFF_COMMITTED_TCB: usize = 0x1E0;
const OFF_CURRENT_BUILD: usize = 0x1E8;
const OFF_CURRENT_MINOR: usize = 0x1E9;
const OFF_CURRENT_MAJOR: usize = 0x1EA;
const OFF_COMMITTED_BUILD: usize = 0x1EC;
const OFF_COMMITTED_MINOR: usize = 0x1ED;
const OFF_COMMITTED_MAJOR: usize = 0x1EE;
const OFF_LAUNCH_TCB: usize = 0x1F0;
const OFF_SIGNATURE: usize = 0x2A0;

// ============================================================================
// Certificate Type (from author_key_en field)
// ============================================================================

/// VEK type determined from attestation report
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VekType {
    /// Versioned Chip Endorsement Key
    Vcek,
    /// Versioned Loaded Endorsement Key
    Vlek,
}

// ============================================================================
// Guest Policy
// ============================================================================

/// Guest policy flags from the attestation report
#[derive(Debug, Clone, Copy, Default)]
pub struct GuestPolicy {
    /// Raw 64-bit policy value
    pub raw: u64,
    /// ABI minor version
    pub abi_minor: u8,
    /// ABI major version
    pub abi_major: u8,
    /// SMT (Simultaneous Multi-Threading) allowed
    pub smt_allowed: bool,
    /// Migration agent allowed
    pub migrate_ma: bool,
    /// Debugging allowed
    pub debug: bool,
    /// Single socket only
    pub single_socket: bool,
}

impl GuestPolicy {
    /// Parse guest policy from raw 64-bit value
    pub fn from_raw(raw: u64) -> Self {
        Self {
            raw,
            abi_minor: (raw & 0xFF) as u8,
            abi_major: ((raw >> 8) & 0xFF) as u8,
            smt_allowed: (raw >> 16) & 1 == 1,
            migrate_ma: (raw >> 18) & 1 == 1,
            debug: (raw >> 19) & 1 == 1,
            single_socket: (raw >> 20) & 1 == 1,
        }
    }
}

// ============================================================================
// Platform Info
// ============================================================================

/// Platform info flags from the attestation report
#[derive(Debug, Clone, Copy, Default)]
pub struct PlatformInfo {
    /// Raw 64-bit value
    pub raw: u64,
    /// SMT enabled
    pub smt_enabled: bool,
    /// TSME enabled (Transparent SME)
    pub tsme_enabled: bool,
}

impl PlatformInfo {
    /// Parse platform info from raw 64-bit value
    pub fn from_raw(raw: u64) -> Self {
        Self {
            raw,
            smt_enabled: raw & 1 == 1,
            tsme_enabled: (raw >> 1) & 1 == 1,
        }
    }
}

// ============================================================================
// Attestation Report
// ============================================================================

/// Parsed AMD SEV-SNP Attestation Report
///
/// This structure matches the Cairo `AttestationReport` struct.
/// See AMD SEV-SNP specification Table 23.
#[derive(Debug, Clone)]
pub struct AttestationReport {
    /// Version of the attestation report (should be 2)
    pub version: u32,
    /// Guest Security Version Number
    pub guest_svn: u32,
    /// Guest policy
    pub policy: GuestPolicy,
    /// Family ID (16 bytes)
    pub family_id: [u8; 16],
    /// Image ID (16 bytes)
    pub image_id: [u8; 16],
    /// Virtual Machine Privilege Level (0-3)
    pub vmpl: u32,
    /// Signature algorithm (1 = ECDSA P-384 with SHA-384)
    pub signature_algo: u32,
    /// Current TCB version
    pub current_tcb: TcbVersion,
    /// Platform info
    pub platform_info: PlatformInfo,
    /// Author key enabled flags
    pub author_key_en: u32,
    /// Report data (64 bytes) - guest-provided data
    pub report_data: [u8; 64],
    /// Measurement (48 bytes) - launch digest
    pub measurement: [u8; 48],
    /// Host data (32 bytes)
    pub host_data: [u8; 32],
    /// ID key digest (48 bytes)
    pub id_key_digest: [u8; 48],
    /// Author key digest (48 bytes)
    pub author_key_digest: [u8; 48],
    /// Report ID (32 bytes)
    pub report_id: [u8; 32],
    /// Report ID MA (32 bytes)
    pub report_id_ma: [u8; 32],
    /// Reported TCB version
    pub reported_tcb: TcbVersion,
    /// Chip ID (64 bytes)
    pub chip_id: ChipId,
    /// Committed TCB version
    pub committed_tcb: TcbVersion,
    /// Current build number
    pub current_build: u8,
    /// Current minor version
    pub current_minor: u8,
    /// Current major version
    pub current_major: u8,
    /// Committed build number
    pub committed_build: u8,
    /// Committed minor version
    pub committed_minor: u8,
    /// Committed major version
    pub committed_major: u8,
    /// Launch TCB version
    pub launch_tcb: TcbVersion,
    /// Signature (512 bytes)
    pub signature: [u8; SIGNATURE_SIZE],
}

impl AttestationReport {
    /// Parse an attestation report from raw bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        if data.len() < ATTESTATION_REPORT_SIZE {
            return Err(Error::CertificateParse(format!(
                "Attestation report too short: {} bytes, expected {}",
                data.len(),
                ATTESTATION_REPORT_SIZE
            )));
        }

        Ok(Self {
            version: read_u32_le(data, OFF_VERSION),
            guest_svn: read_u32_le(data, OFF_GUEST_SVN),
            policy: GuestPolicy::from_raw(read_u64_le(data, OFF_POLICY)),
            family_id: read_bytes::<16>(data, OFF_FAMILY_ID),
            image_id: read_bytes::<16>(data, OFF_IMAGE_ID),
            vmpl: read_u32_le(data, OFF_VMPL),
            signature_algo: read_u32_le(data, OFF_SIG_ALGO),
            current_tcb: read_tcb(data, OFF_CURRENT_TCB),
            platform_info: PlatformInfo::from_raw(read_u64_le(data, OFF_PLAT_INFO)),
            author_key_en: read_u32_le(data, OFF_AUTHOR_KEY_EN),
            report_data: read_bytes::<64>(data, OFF_REPORT_DATA),
            measurement: read_bytes::<48>(data, OFF_MEASUREMENT),
            host_data: read_bytes::<32>(data, OFF_HOST_DATA),
            id_key_digest: read_bytes::<48>(data, OFF_ID_KEY_DIGEST),
            author_key_digest: read_bytes::<48>(data, OFF_AUTHOR_KEY_DIGEST),
            report_id: read_bytes::<32>(data, OFF_REPORT_ID),
            report_id_ma: read_bytes::<32>(data, OFF_REPORT_ID_MA),
            reported_tcb: read_tcb(data, OFF_REPORTED_TCB),
            chip_id: ChipId::new(read_bytes::<64>(data, OFF_CHIP_ID)),
            committed_tcb: read_tcb(data, OFF_COMMITTED_TCB),
            current_build: data[OFF_CURRENT_BUILD],
            current_minor: data[OFF_CURRENT_MINOR],
            current_major: data[OFF_CURRENT_MAJOR],
            committed_build: data[OFF_COMMITTED_BUILD],
            committed_minor: data[OFF_COMMITTED_MINOR],
            committed_major: data[OFF_COMMITTED_MAJOR],
            launch_tcb: read_tcb(data, OFF_LAUNCH_TCB),
            signature: read_bytes::<SIGNATURE_SIZE>(data, OFF_SIGNATURE),
        })
    }

    /// Get the VEK type (VCEK or VLEK) from this report
    pub fn vek_type(&self) -> VekType {
        // Bits [2:4] of author_key_en indicate signer type
        // 0 = VCEK, 1 = VLEK
        let signer_type = (self.author_key_en >> 2) & 0x7;
        if signer_type == 0 {
            VekType::Vcek
        } else {
            VekType::Vlek
        }
    }

    /// Get the processor type based on current version info
    /// This is a heuristic based on the version numbers
    pub fn processor_type(&self) -> Option<ProcessorType> {
        // Milan: major version typically 1
        // Genoa: major version typically 2+
        match self.current_major {
            1 => Some(ProcessorType::Milan),
            _ => Some(ProcessorType::Genoa), // Default to Genoa for newer
        }
    }

    /// Check if this report has a valid version
    pub fn is_valid_version(&self) -> bool {
        self.version == 2
    }

    /// Check if debugging is allowed
    pub fn is_debug(&self) -> bool {
        self.policy.debug
    }
}

// ============================================================================
// Helper functions
// ============================================================================

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u64_le(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

fn read_bytes<const N: usize>(data: &[u8], offset: usize) -> [u8; N] {
    let mut arr = [0u8; N];
    arr.copy_from_slice(&data[offset..offset + N]);
    arr
}

fn read_tcb(data: &[u8], offset: usize) -> TcbVersion {
    TcbVersion::from_bytes(&read_bytes::<8>(data, offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample attestation report from go-sev-guest testdata
    const SAMPLE_REPORT: &[u8] = include_bytes!("testdata/attestation.bin");

    #[test]
    fn test_sample_report_size() {
        assert_eq!(
            SAMPLE_REPORT.len(),
            ATTESTATION_REPORT_SIZE,
            "Sample report should be {} bytes",
            ATTESTATION_REPORT_SIZE
        );
    }

    #[test]
    fn test_parse_sample_report() {
        let report = AttestationReport::from_bytes(SAMPLE_REPORT)
            .expect("Failed to parse sample attestation report");

        // Verify version
        assert_eq!(report.version, 2, "Version should be 2");
        assert!(report.is_valid_version());

        // Print parsed fields
        println!("=== Parsed Attestation Report ===");
        println!("Version: {}", report.version);
        println!("Guest SVN: {}", report.guest_svn);
        println!("Policy (raw): 0x{:016x}", report.policy.raw);
        println!(
            "  ABI: {}.{}",
            report.policy.abi_major, report.policy.abi_minor
        );
        println!("  SMT allowed: {}", report.policy.smt_allowed);
        println!("  Debug: {}", report.policy.debug);
        println!("VMPL: {}", report.vmpl);
        println!("Signature algo: {}", report.signature_algo);
        println!("VEK type: {:?}", report.vek_type());
    }

    #[test]
    fn test_parse_tcb_versions() {
        let report = AttestationReport::from_bytes(SAMPLE_REPORT)
            .expect("Failed to parse sample attestation report");

        println!("=== TCB Versions ===");
        println!(
            "Current TCB: bl={}, tee={}, snp={}, ucode={}",
            report.current_tcb.bootloader,
            report.current_tcb.tee,
            report.current_tcb.snp,
            report.current_tcb.microcode
        );
        println!(
            "Reported TCB: bl={}, tee={}, snp={}, ucode={}",
            report.reported_tcb.bootloader,
            report.reported_tcb.tee,
            report.reported_tcb.snp,
            report.reported_tcb.microcode
        );
        println!(
            "Committed TCB: bl={}, tee={}, snp={}, ucode={}",
            report.committed_tcb.bootloader,
            report.committed_tcb.tee,
            report.committed_tcb.snp,
            report.committed_tcb.microcode
        );
        println!(
            "Launch TCB: bl={}, tee={}, snp={}, ucode={}",
            report.launch_tcb.bootloader,
            report.launch_tcb.tee,
            report.launch_tcb.snp,
            report.launch_tcb.microcode
        );
    }

    #[test]
    fn test_parse_chip_id() {
        let report = AttestationReport::from_bytes(SAMPLE_REPORT)
            .expect("Failed to parse sample attestation report");

        println!("=== Chip ID ===");
        println!("Chip ID: {}", report.chip_id);

        // Chip ID should not be all zeros (in a real report)
        let all_zeros = report.chip_id.as_bytes().iter().all(|&b| b == 0);
        println!("Chip ID is all zeros: {}", all_zeros);
    }

    #[test]
    fn test_parse_measurement() {
        let report = AttestationReport::from_bytes(SAMPLE_REPORT)
            .expect("Failed to parse sample attestation report");

        println!("=== Measurement (Launch Digest) ===");
        println!("Measurement: {}", hex::encode(&report.measurement));

        // Measurement should not be all zeros
        let all_zeros = report.measurement.iter().all(|&b| b == 0);
        println!("Measurement is all zeros: {}", all_zeros);
    }

    #[test]
    fn test_parse_report_data() {
        let report = AttestationReport::from_bytes(SAMPLE_REPORT)
            .expect("Failed to parse sample attestation report");

        println!("=== Report Data (Guest-provided) ===");
        println!("Report data: {}", hex::encode(&report.report_data));
    }

    #[test]
    fn test_parse_platform_info() {
        let report = AttestationReport::from_bytes(SAMPLE_REPORT)
            .expect("Failed to parse sample attestation report");

        println!("=== Platform Info ===");
        println!("Platform info (raw): 0x{:016x}", report.platform_info.raw);
        println!("SMT enabled: {}", report.platform_info.smt_enabled);
        println!("TSME enabled: {}", report.platform_info.tsme_enabled);
    }

    #[test]
    fn test_parse_version_info() {
        let report = AttestationReport::from_bytes(SAMPLE_REPORT)
            .expect("Failed to parse sample attestation report");

        println!("=== Version Info ===");
        println!(
            "Current: {}.{}.{}",
            report.current_major, report.current_minor, report.current_build
        );
        println!(
            "Committed: {}.{}.{}",
            report.committed_major, report.committed_minor, report.committed_build
        );
    }

    #[test]
    fn test_signature_not_empty() {
        let report = AttestationReport::from_bytes(SAMPLE_REPORT)
            .expect("Failed to parse sample attestation report");

        // Signature should not be all zeros
        let all_zeros = report.signature.iter().all(|&b| b == 0);
        assert!(!all_zeros, "Signature should not be all zeros");

        println!("=== Signature ===");
        println!(
            "Signature (first 64 bytes): {}",
            hex::encode(&report.signature[..64])
        );
    }

    #[test]
    fn test_invalid_report_size() {
        let short_data = vec![0u8; 100];
        let result = AttestationReport::from_bytes(&short_data);
        assert!(result.is_err(), "Should fail with short data");
    }
}
