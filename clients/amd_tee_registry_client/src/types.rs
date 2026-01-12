//! AMD SEV-SNP Types
//!
//! These types mirror the Cairo types in `contracts/amd_tee_registry/src/tee_types.cairo`
//! for consistency across the codebase.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

// ============================================================================
// Enums
// ============================================================================

/// Certificate types for AMD SEV-SNP
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CertType {
    /// Versioned Chip Endorsement Key
    VCEK,
    /// Versioned Loaded Endorsement Key (NOT available on KDS - only from device)
    VLEK,
    /// AMD SEV Signing Key
    ASK,
    /// AMD Root Signing Key
    ARK,
}

impl fmt::Display for CertType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CertType::VCEK => write!(f, "VCEK"),
            CertType::VLEK => write!(f, "VLEK"),
            CertType::ASK => write!(f, "ASK"),
            CertType::ARK => write!(f, "ARK"),
        }
    }
}

/// AMD EPYC Processor types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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

impl ProcessorType {
    /// Returns the KDS API path segment for this processor type
    pub fn as_kds_path(&self) -> &'static str {
        match self {
            ProcessorType::Milan => "Milan",
            ProcessorType::Genoa => "Genoa",
            ProcessorType::Bergamo => "Bergamo",
            ProcessorType::Siena => "Siena",
        }
    }

    /// Parse processor type from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "milan" => Some(ProcessorType::Milan),
            "genoa" => Some(ProcessorType::Genoa),
            "bergamo" => Some(ProcessorType::Bergamo),
            "siena" => Some(ProcessorType::Siena),
            _ => None,
        }
    }
}

impl fmt::Display for ProcessorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_kds_path())
    }
}

// ============================================================================
// TCB Struct
// ============================================================================

/// TCB_VERSION structure (8 bytes)
/// See Table 3 of AMD SEV-SNP specification
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcbVersion {
    pub bootloader: u8,
    pub tee: u8,
    /// Reserved bytes (4 bytes packed)
    pub reserved: u32,
    pub snp: u8,
    pub microcode: u8,
}

impl TcbVersion {
    /// Create a new TcbVersion from raw bytes (8 bytes)
    pub fn from_bytes(bytes: &[u8; 8]) -> Self {
        Self {
            bootloader: bytes[0],
            tee: bytes[1],
            reserved: u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            snp: bytes[6],
            microcode: bytes[7],
        }
    }

    /// Convert to raw bytes (8 bytes)
    pub fn to_bytes(&self) -> [u8; 8] {
        let reserved_bytes = self.reserved.to_le_bytes();
        [
            self.bootloader,
            self.tee,
            reserved_bytes[0],
            reserved_bytes[1],
            reserved_bytes[2],
            reserved_bytes[3],
            self.snp,
            self.microcode,
        ]
    }
}

// ============================================================================
// Chip ID
// ============================================================================

/// Chip ID (64 bytes)
/// Unique identifier for the AMD processor chip
#[derive(Clone, PartialEq, Eq)]
pub struct ChipId([u8; 64]);

impl ChipId {
    /// Create from a 64-byte array
    pub fn new(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    /// Create from a byte slice
    pub fn from_slice(bytes: &[u8]) -> Result<Self, crate::Error> {
        if bytes.len() != 64 {
            return Err(crate::Error::InvalidChipId(bytes.len()));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }

    /// Parse from hex string (128 hex characters)
    pub fn from_hex(hex_str: &str) -> Result<Self, crate::Error> {
        let bytes =
            hex::decode(hex_str).map_err(|_| crate::Error::InvalidChipId(hex_str.len() / 2))?;
        Self::from_slice(&bytes)
    }

    /// Convert to hex string (lowercase)
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }

    /// Get the raw bytes
    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl fmt::Debug for ChipId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChipId({})", self.to_hex())
    }
}

impl fmt::Display for ChipId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for ChipId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as hex string
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for ChipId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = String::deserialize(deserializer)?;
        ChipId::from_hex(&hex_str).map_err(serde::de::Error::custom)
    }
}

// ============================================================================
// Certificate Chain
// ============================================================================

/// AMD certificate chain containing ARK and ASK
#[derive(Debug, Clone)]
pub struct CertChain {
    /// AMD Root Key certificate (DER encoded)
    pub ark: Vec<u8>,
    /// AMD SEV Key certificate (DER encoded)
    pub ask: Vec<u8>,
}

/// VCEK certificate with metadata
#[derive(Debug, Clone)]
pub struct VcekCert {
    /// VCEK certificate (DER encoded)
    pub der: Vec<u8>,
    /// Processor type this VCEK is for
    pub processor: ProcessorType,
    /// Chip ID this VCEK is bound to
    pub chip_id: ChipId,
    /// TCB version this VCEK is bound to
    pub tcb: TcbVersion,
}

/// Complete certificate bundle for attestation verification
#[derive(Debug, Clone)]
pub struct AttestationCerts {
    /// Certificate chain (ARK + ASK)
    pub chain: CertChain,
    /// VCEK certificate
    pub vcek: VcekCert,
}

// ============================================================================
// VCEK Request Parameters
// ============================================================================

/// Parameters for requesting a VCEK certificate from KDS
#[derive(Debug, Clone)]
pub struct VcekRequest {
    /// Processor type
    pub processor: ProcessorType,
    /// Chip ID (64 bytes)
    pub chip_id: ChipId,
    /// TCB version from attestation report
    pub tcb: TcbVersion,
}

impl VcekRequest {
    /// Create a new VCEK request
    pub fn new(processor: ProcessorType, chip_id: ChipId, tcb: TcbVersion) -> Self {
        Self {
            processor,
            chip_id,
            tcb,
        }
    }

    /// Build the query parameters for the KDS API
    pub fn query_params(&self) -> Vec<(&'static str, String)> {
        vec![
            ("blSPL", self.tcb.bootloader.to_string()),
            ("teeSPL", self.tcb.tee.to_string()),
            ("snpSPL", self.tcb.snp.to_string()),
            ("ucodeSPL", self.tcb.microcode.to_string()),
        ]
    }
}
