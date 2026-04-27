// AMD SEV-SNP Attestation Types
// Based on AMD SEV-SNP ABI Specification (Table 3 and Table 23)
// Uses Span<u32> as input (296 words = 1184 bytes)

use core::integer::u512;
use super::byte_utils::{
    Bytes24, Bytes48, get_bytes24_at, get_bytes48_at, get_u128_at, get_u256_at, get_u32_at,
    get_u512_at, get_u64_at, get_u8_from_word, slice_u32_span,
};

// ============================================================================
// Enums
// ============================================================================

/// Certificate types for AMD SEV-SNP
#[derive(Drop, Copy, PartialEq, Debug)]
pub enum CertType {
    /// Versioned Chip Endorsement Key
    VCEK,
    /// Versioned Loaded Endorsement Key
    VLEK,
    /// AMD SEV Signing Key
    ASK,
    /// AMD Root Signing Key
    ARK,
}

/// AMD EPYC Processor types
#[derive(Drop, Copy, PartialEq, Debug, Serde, starknet::Store, Hash)]
pub enum ProcessorType {
    /// 7003 series AMD EPYC Processor
    #[default]
    Milan,
    /// 9004 series AMD EPYC Processor
    Genoa,
    /// 97x4 series AMD EPYC Processor
    Bergamo,
    /// 8004 series AMD EPYC Processor
    Siena,
}

// /// ZK Co-Processor types for proof verification
// #[derive(Drop, Copy, PartialEq, Debug)]
// pub enum ZkCoProcessorType {
//     None,
//     RiscZero,
//     Succinct,
//     Pico,
// }

/// Verification result status
#[derive(Drop, Copy, PartialEq, Debug, Serde)]
pub enum VerificationResult {
    /// Verification succeeded
    Success,
    /// Root certificate is not trusted
    RootCertNotTrusted,
    /// One or more intermediate certificates are not trusted
    IntermediateCertsNotTrusted,
    /// Attestation timestamp is outside acceptable range
    InvalidTimestamp,
}

// ============================================================================
// Verifier Structs
// ============================================================================
// struct VerifierInput {
//     uint64 timestamp;
//     uint8 trustedCertsPrefixLen;
//     bytes rawReport;
//     bytes[] vekDerChain;
// }
/// Input for the verifier
#[derive(Drop, Debug)]
pub struct VerifierInput {
    /// Unix timestamp
    pub timestamp: u64,
    /// Length of trusted certificates prefix
    pub trusted_certs_prefix_len: u8,
    /// Raw attestation report as u32 words
    pub raw_report: Span<u32>,
    /// VEK DER certificate chain (array of certificate byte arrays as u32 words)
    pub vek_der_chain: Array<Span<u32>>,
}

/// Journal output from the verifier
#[derive(Drop, Debug, PartialEq, Serde)]
pub struct VerifierJournal {
    /// Verification result
    pub result: VerificationResult,
    /// Unix timestamp
    pub timestamp: u64,
    /// Processor model (can be converted to ProcessorType)
    pub processor_model: u8,
    /// Raw attestation report as u32 words
    pub raw_report: Span<u32>,
    /// Certificate hashes (bytes32 = u256)
    pub certs: Array<u256>,
    /// Certificate serial numbers (uint160 fits in felt252)
    pub cert_serials: Array<felt252>,
    /// Length of trusted certificates prefix
    pub trusted_certs_prefix_len: u8,
    /// Commitment to verified storage. 0 when no storage proof.
    pub storage_commitment: felt252,
    /// Fork block number (0 = non-fork mode).
    pub fork_block_number: u64,
    /// Shard end block number where ShardFinished event was proven (0 = no event proof).
    pub end_block_number: u64,
}

// ============================================================================
// TCB Struct
// ============================================================================

/// TCB_VERSION structure (8 bytes = 2 u32 words)
/// See Table 3 of AMD SEV-SNP specification
#[derive(Drop, Copy, Default, PartialEq, Debug)]
pub struct TcbVersion {
    pub bootloader: u8,
    pub tee: u8,
    /// Reserved bytes packed as u32 (4 bytes)
    pub reserved: u32,
    pub snp: u8,
    pub microcode: u8,
}

// ============================================================================
// Attestation Report Struct
// ============================================================================

/// Attestation Report structure
/// See Table 23 of AMD SEV-SNP specification (total size is 1184 bytes = 296 u32 words)
#[derive(Drop)]
pub struct AttestationReport {
    /// Version of the attestation report (4 bytes)
    pub version: u32,
    /// Guest SVN (4 bytes)
    pub guest_svn: u32,
    /// Guest policy raw (8 bytes)
    pub guest_policy_raw: u64,
    /// Family ID (16 bytes)
    pub family_id: u128,
    /// Image ID (16 bytes)
    pub image_id: u128,
    /// VMPL (4 bytes)
    pub vmpl: u32,
    /// Signature algorithm (4 bytes)
    pub sig_algo: u32,
    /// Current TCB (8 bytes)
    pub current_tcb: TcbVersion,
    /// Platform info raw (8 bytes)
    pub plat_info_raw: u64,
    /// Author key enabled (4 bytes)
    pub author_key_en: u32,
    /// Reserved (4 bytes)
    pub reserved0: u32,
    /// Report data (64 bytes)
    pub report_data: u512,
    /// Measurement (48 bytes)
    pub measurement: Bytes48,
    /// Host data (32 bytes)
    pub host_data: u256,
    /// ID key digest (48 bytes)
    pub id_key_digest: Bytes48,
    /// Author key digest (48 bytes)
    pub author_key_digest: Bytes48,
    /// Report ID (32 bytes)
    pub report_id: u256,
    /// Report ID MA (32 bytes)
    pub report_id_ma: u256,
    /// Reported TCB (8 bytes)
    pub reported_tcb: TcbVersion,
    /// Reserved (24 bytes)
    pub reserved1: Bytes24,
    /// Chip ID (64 bytes)
    pub chip_id: u512,
    /// Committed TCB (8 bytes)
    pub committed_tcb: TcbVersion,
    /// Current build number (1 byte)
    pub current_build: u8,
    /// Current minor version (1 byte)
    pub current_minor: u8,
    /// Current major version (1 byte)
    pub current_major: u8,
    /// Reserved (1 byte)
    pub reserved2: u8,
    /// Committed build number (1 byte)
    pub committed_build: u8,
    /// Committed minor version (1 byte)
    pub committed_minor: u8,
    /// Committed major version (1 byte)
    pub committed_major: u8,
    /// Reserved (1 byte)
    pub reserved3: u8,
    /// Launch TCB (8 bytes)
    pub launch_tcb: TcbVersion,
    /// Signature (512 bytes = 128 u32 words)
    pub signature: Span<u32>,
}

// ============================================================================
// Constants for u32 word offsets (byte_offset / 4)
// ============================================================================

/// Total size of attestation report in u32 words (1184 bytes / 4)
pub const ATTESTATION_REPORT_SIZE_U32: u32 = 296;

// Field offsets in u32 words
pub const OFF_VERSION: u32 = 0; // 0x00 / 4
pub const OFF_GUEST_SVN: u32 = 1; // 0x04 / 4
pub const OFF_GUEST_POLICY: u32 = 2; // 0x08 / 4
pub const OFF_FAMILY_ID: u32 = 4; // 0x10 / 4
pub const OFF_IMAGE_ID: u32 = 8; // 0x20 / 4
pub const OFF_VMPL: u32 = 12; // 0x30 / 4
pub const OFF_SIG_ALGO: u32 = 13; // 0x34 / 4
pub const OFF_CURRENT_TCB: u32 = 14; // 0x38 / 4
pub const OFF_PLAT_INFO: u32 = 16; // 0x40 / 4
pub const OFF_AUTHOR_KEY_EN: u32 = 18; // 0x48 / 4
pub const OFF_RESERVED0: u32 = 19; // 0x4C / 4
pub const OFF_REPORT_DATA: u32 = 20; // 0x50 / 4
pub const OFF_MEASUREMENT: u32 = 36; // 0x90 / 4
pub const OFF_HOST_DATA: u32 = 48; // 0xC0 / 4
pub const OFF_ID_KEY_DIGEST: u32 = 56; // 0xE0 / 4
pub const OFF_AUTHOR_KEY_DIGEST: u32 = 68; // 0x110 / 4
pub const OFF_REPORT_ID: u32 = 80; // 0x140 / 4
pub const OFF_REPORT_ID_MA: u32 = 88; // 0x160 / 4
pub const OFF_REPORTED_TCB: u32 = 96; // 0x180 / 4
pub const OFF_RESERVED1: u32 = 98; // 0x188 / 4
pub const OFF_CHIP_ID: u32 = 104; // 0x1A0 / 4
pub const OFF_COMMITTED_TCB: u32 = 120; // 0x1E0 / 4
pub const OFF_VERSION_INFO: u32 = 122; // 0x1E8 / 4 (current/committed build/minor/major)
pub const OFF_LAUNCH_TCB: u32 = 124; // 0x1F0 / 4
pub const OFF_RESERVED4: u32 = 126; // 0x1F8 / 4
pub const OFF_SIGNATURE: u32 = 168; // 0x2A0 / 4

// ============================================================================
// Helper functions for TCB parsing
// ============================================================================

/// Read TcbVersion from 2 consecutive u32 words
/// TCB layout: [bootloader, tee, reserved[0], reserved[1]] [reserved[2], reserved[3], snp,
/// microcode]
pub fn get_tcb_version_at(data: Span<u32>, index: u32) -> TcbVersion {
    let word0 = *data.at(index);
    let word1 = *data.at(index + 1);

    // Extract bytes from word0 using division and modulo
    // word0 = bootloader + tee*256 + reserved[0]*65536 + reserved[1]*16777216
    let bootloader = word0 % 256;
    let rest0 = word0 / 256;
    let tee = rest0 % 256;
    let reserved_low = rest0 / 256; // reserved[0] + reserved[1]*256

    // word1 = reserved[2] + reserved[3]*256 + snp*65536 + microcode*16777216
    let reserved_high = word1 % 65536; // reserved[2] + reserved[3]*256
    let rest1 = word1 / 65536;
    let snp = rest1 % 256;
    let microcode = rest1 / 256;

    // Pack reserved bytes into u32
    let reserved: u32 = reserved_low + (reserved_high * 65536);

    TcbVersion {
        bootloader: bootloader.try_into().unwrap(),
        tee: tee.try_into().unwrap(),
        reserved,
        snp: snp.try_into().unwrap(),
        microcode: microcode.try_into().unwrap(),
    }
}

// ============================================================================
// RawAttestationReport wrapper with lazy field accessors
// ============================================================================

#[derive(Drop, Copy)]
pub struct RawAttestationReport {
    pub raw: Span<u32>,
}

#[generate_trait]
pub impl RawAttestationReportImpl of RawAttestationReportTrait {
    /// Create a new RawAttestationReport from a Span<u32>
    fn new(raw: Span<u32>) -> RawAttestationReport {
        assert!(raw.len() >= ATTESTATION_REPORT_SIZE_U32, "Attestation report too short");
        RawAttestationReport { raw }
    }

    /// Convert to full AttestationReport struct
    fn to_attestation_report(self: @RawAttestationReport) -> AttestationReport {
        AttestationReport {
            version: self.version(),
            guest_svn: self.guest_svn(),
            guest_policy_raw: self.guest_policy_raw(),
            family_id: self.family_id(),
            image_id: self.image_id(),
            vmpl: self.vmpl(),
            sig_algo: self.sig_algo(),
            current_tcb: self.current_tcb(),
            plat_info_raw: self.plat_info_raw(),
            author_key_en: self.author_key_en(),
            reserved0: self.reserved0(),
            report_data: self.report_data(),
            measurement: self.measurement(),
            host_data: self.host_data(),
            id_key_digest: self.id_key_digest(),
            author_key_digest: self.author_key_digest(),
            report_id: self.report_id(),
            report_id_ma: self.report_id_ma(),
            reported_tcb: self.reported_tcb(),
            reserved1: self.reserved1(),
            chip_id: self.chip_id(),
            committed_tcb: self.committed_tcb(),
            current_build: self.current_build(),
            current_minor: self.current_minor(),
            current_major: self.current_major(),
            reserved2: self.reserved2(),
            committed_build: self.committed_build(),
            committed_minor: self.committed_minor(),
            committed_major: self.committed_major(),
            reserved3: self.reserved3(),
            launch_tcb: self.launch_tcb(),
            signature: self.signature(),
        }
    }

    // ========================================================================
    // Field accessors with correct u32 offsets
    // ========================================================================

    /// Version at offset 0 (1 u32)
    fn version(self: @RawAttestationReport) -> u32 {
        get_u32_at(*self.raw, OFF_VERSION)
    }

    /// Guest SVN at offset 1 (1 u32)
    fn guest_svn(self: @RawAttestationReport) -> u32 {
        get_u32_at(*self.raw, OFF_GUEST_SVN)
    }

    /// Guest policy at offset 2 (2 u32 = 8 bytes)
    fn guest_policy_raw(self: @RawAttestationReport) -> u64 {
        get_u64_at(*self.raw, OFF_GUEST_POLICY)
    }

    /// Family ID at offset 4 (4 u32 = 16 bytes)
    fn family_id(self: @RawAttestationReport) -> u128 {
        get_u128_at(*self.raw, OFF_FAMILY_ID)
    }

    /// Image ID at offset 8 (4 u32 = 16 bytes)
    fn image_id(self: @RawAttestationReport) -> u128 {
        get_u128_at(*self.raw, OFF_IMAGE_ID)
    }

    /// VMPL at offset 12 (1 u32)
    fn vmpl(self: @RawAttestationReport) -> u32 {
        get_u32_at(*self.raw, OFF_VMPL)
    }

    /// Signature algorithm at offset 13 (1 u32)
    fn sig_algo(self: @RawAttestationReport) -> u32 {
        get_u32_at(*self.raw, OFF_SIG_ALGO)
    }

    /// Current TCB at offset 14 (2 u32 = 8 bytes)
    fn current_tcb(self: @RawAttestationReport) -> TcbVersion {
        get_tcb_version_at(*self.raw, OFF_CURRENT_TCB)
    }

    /// Platform info at offset 16 (2 u32 = 8 bytes)
    fn plat_info_raw(self: @RawAttestationReport) -> u64 {
        get_u64_at(*self.raw, OFF_PLAT_INFO)
    }

    /// Author key enabled at offset 18 (1 u32)
    fn author_key_en(self: @RawAttestationReport) -> u32 {
        get_u32_at(*self.raw, OFF_AUTHOR_KEY_EN)
    }

    /// Reserved0 at offset 19 (1 u32)
    fn reserved0(self: @RawAttestationReport) -> u32 {
        get_u32_at(*self.raw, OFF_RESERVED0)
    }

    /// Report data at offset 20 (16 u32 = 64 bytes)
    fn report_data(self: @RawAttestationReport) -> u512 {
        get_u512_at(*self.raw, OFF_REPORT_DATA)
    }

    /// Measurement at offset 36 (12 u32 = 48 bytes)
    fn measurement(self: @RawAttestationReport) -> Bytes48 {
        get_bytes48_at(*self.raw, OFF_MEASUREMENT)
    }

    /// Host data at offset 48 (8 u32 = 32 bytes)
    fn host_data(self: @RawAttestationReport) -> u256 {
        get_u256_at(*self.raw, OFF_HOST_DATA)
    }

    /// ID key digest at offset 56 (12 u32 = 48 bytes)
    fn id_key_digest(self: @RawAttestationReport) -> Bytes48 {
        get_bytes48_at(*self.raw, OFF_ID_KEY_DIGEST)
    }

    /// Author key digest at offset 68 (12 u32 = 48 bytes)
    fn author_key_digest(self: @RawAttestationReport) -> Bytes48 {
        get_bytes48_at(*self.raw, OFF_AUTHOR_KEY_DIGEST)
    }

    /// Report ID at offset 80 (8 u32 = 32 bytes)
    fn report_id(self: @RawAttestationReport) -> u256 {
        get_u256_at(*self.raw, OFF_REPORT_ID)
    }

    /// Report ID MA at offset 88 (8 u32 = 32 bytes)
    fn report_id_ma(self: @RawAttestationReport) -> u256 {
        get_u256_at(*self.raw, OFF_REPORT_ID_MA)
    }

    /// Reported TCB at offset 96 (2 u32 = 8 bytes)
    fn reported_tcb(self: @RawAttestationReport) -> TcbVersion {
        get_tcb_version_at(*self.raw, OFF_REPORTED_TCB)
    }

    /// Reserved1 at offset 98 (6 u32 = 24 bytes)
    fn reserved1(self: @RawAttestationReport) -> Bytes24 {
        get_bytes24_at(*self.raw, OFF_RESERVED1)
    }

    /// Chip ID at offset 104 (16 u32 = 64 bytes)
    fn chip_id(self: @RawAttestationReport) -> u512 {
        get_u512_at(*self.raw, OFF_CHIP_ID)
    }

    /// Committed TCB at offset 120 (2 u32 = 8 bytes)
    fn committed_tcb(self: @RawAttestationReport) -> TcbVersion {
        get_tcb_version_at(*self.raw, OFF_COMMITTED_TCB)
    }

    /// Current build at offset 122, byte 0
    fn current_build(self: @RawAttestationReport) -> u8 {
        let word = get_u32_at(*self.raw, OFF_VERSION_INFO);
        get_u8_from_word(word, 0)
    }

    /// Current minor at offset 122, byte 1
    fn current_minor(self: @RawAttestationReport) -> u8 {
        let word = get_u32_at(*self.raw, OFF_VERSION_INFO);
        get_u8_from_word(word, 1)
    }

    /// Current major at offset 122, byte 2
    fn current_major(self: @RawAttestationReport) -> u8 {
        let word = get_u32_at(*self.raw, OFF_VERSION_INFO);
        get_u8_from_word(word, 2)
    }

    /// Reserved2 at offset 122, byte 3
    fn reserved2(self: @RawAttestationReport) -> u8 {
        let word = get_u32_at(*self.raw, OFF_VERSION_INFO);
        get_u8_from_word(word, 3)
    }

    /// Committed build at offset 123, byte 0
    fn committed_build(self: @RawAttestationReport) -> u8 {
        let word = get_u32_at(*self.raw, OFF_VERSION_INFO + 1);
        get_u8_from_word(word, 0)
    }

    /// Committed minor at offset 123, byte 1
    fn committed_minor(self: @RawAttestationReport) -> u8 {
        let word = get_u32_at(*self.raw, OFF_VERSION_INFO + 1);
        get_u8_from_word(word, 1)
    }

    /// Committed major at offset 123, byte 2
    fn committed_major(self: @RawAttestationReport) -> u8 {
        let word = get_u32_at(*self.raw, OFF_VERSION_INFO + 1);
        get_u8_from_word(word, 2)
    }

    /// Reserved3 at offset 123, byte 3
    fn reserved3(self: @RawAttestationReport) -> u8 {
        let word = get_u32_at(*self.raw, OFF_VERSION_INFO + 1);
        get_u8_from_word(word, 3)
    }

    /// Launch TCB at offset 124 (2 u32 = 8 bytes)
    fn launch_tcb(self: @RawAttestationReport) -> TcbVersion {
        get_tcb_version_at(*self.raw, OFF_LAUNCH_TCB)
    }

    /// Signature at offset 168 (128 u32 = 512 bytes)
    fn signature(self: @RawAttestationReport) -> Span<u32> {
        slice_u32_span(*self.raw, OFF_SIGNATURE, 128)
    }

    /// Determine the signing VEK type without parsing entire data
    fn get_vek_type(self: @RawAttestationReport) -> CertType {
        let author_key_en = self.author_key_en();
        // Extract signer type from bits [2:4] using division
        // bits [2:4] = (author_key_en / 4) % 8
        let quotient = author_key_en / 4;
        let signer_type = quotient % 8;

        if signer_type == 0 {
            CertType::VCEK
        } else if signer_type == 1 {
            CertType::VLEK
        } else {
            panic!("Unknown VEK type")
        }
    }
}
