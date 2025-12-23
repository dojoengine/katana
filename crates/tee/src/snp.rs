use tracing::{debug, info};

use crate::error::TeeError;
use crate::provider::TeeProvider;

/// AMD SEV-SNP provider using the Automata Network SEV-SNP SDK.
///
/// This provider uses the /dev/sev-guest device to generate SEV-SNP
/// attestation reports via the Automata Network SDK.
pub struct SevSnpProvider {
    /// SEV-SNP SDK instance
    sev_snp: sev_snp::SevSnp,
}

impl std::fmt::Debug for SevSnpProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SevSnpProvider").finish_non_exhaustive()
    }
}

impl SevSnpProvider {
    /// Create a new SEV-SNP provider.
    ///
    /// # Errors
    ///
    /// Returns `TeeError::NotSupported` if the SEV-SNP SDK initialization fails.
    pub fn new() -> Result<Self, TeeError> {
        let sev_snp = sev_snp::SevSnp::new()
            .map_err(|e| TeeError::NotSupported(format!("Failed to initialize SEV-SNP: {}", e)))?;

        info!(target: "tee::snp", "SEV-SNP provider initialized");

        Ok(Self { sev_snp })
    }
}

impl TeeProvider for SevSnpProvider {
    fn generate_quote(&self, user_data: &[u8; 64]) -> Result<Vec<u8>, TeeError> {
        debug!(target: "tee::snp", "Generating SEV-SNP attestation report");

        // Configure report options with user data and VMPL
        let options =
            sev_snp::device::ReportOptions { report_data: Some(*user_data), vmpl: Some(1) };

        // Generate the attestation report using the SEV-SNP SDK
        let (report, _) =
            self.sev_snp.get_attestation_report_with_options(&options).map_err(|e| {
                TeeError::GenerationFailed(format!("SEV-SNP attestation failed: {}", e))
            })?;

        // Serialize the report structure to bytes
        let report_bytes = bincode::serialize(&report).map_err(|e| {
            TeeError::GenerationFailed(format!("Failed to serialize SEV-SNP report: {}", e))
        })?;

        info!(
            target: "tee::snp",
            report_size = report_bytes.len(),
            "Successfully generated SEV-SNP attestation report"
        );

        Ok(report_bytes)
    }

    fn provider_type(&self) -> &'static str {
        "SEV-SNP"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_quote_fails_on_unsupported_machine() {
        // SevSnp::new() succeeds because it only initializes the KDS client,
        // not the actual SEV-SNP device.
        let provider = SevSnpProvider::new().expect("SevSnpProvider::new should succeed");

        let user_data = [0u8; 64];

        // generate_quote should fail on non-SEV-SNP machines because it attempts
        // to access /dev/sev-guest via Device::new() internally.
        let result = provider.generate_quote(&user_data);

        if let Err(TeeError::GenerationFailed(msg)) = result {
            assert!(msg.contains("SEV-SNP"));
        } else {
            panic!("Expected TeeError::GenerationFailed, got {:?}", result);
        }
    }
}
