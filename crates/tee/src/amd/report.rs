//! Attestation report data types.

use crate::amd::Error;

/// Expected size of AMD SEV-SNP attestation report in bytes.
pub const ATTESTATION_REPORT_SIZE: usize = 1184;

/// Validated attestation report bytes.
#[derive(Debug, Clone)]
pub struct AttestationReportBytes(Vec<u8>);

impl AttestationReportBytes {
    pub fn new(report_bytes: &[u8]) -> Result<Self, Error> {
        if report_bytes.len() != ATTESTATION_REPORT_SIZE {
            return Err(Error::InvalidReportSize {
                expected: ATTESTATION_REPORT_SIZE,
                actual: report_bytes.len(),
            });
        }
        Ok(Self(report_bytes.to_vec()))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for AttestationReportBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}
