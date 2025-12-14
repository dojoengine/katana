use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::error::TeeError;
use crate::provider::TeeProvider;

/// Path to the Linux ConfigFS-TSM interface for TDX attestation.
const TSM_REPORT_PATH: &str = "/sys/kernel/config/tsm/report";

/// TDX (Intel Trust Domain Extensions) provider implementation.
///
/// This provider uses the Linux ConfigFS-TSM interface to generate
/// TDX attestation quotes. The interface is available on systems
/// running in a TDX-enabled VM with proper kernel support.
#[derive(Debug)]
pub struct TdxProvider {
    /// Base path for TSM reports (configurable for testing).
    base_path: PathBuf,
}

impl TdxProvider {
    /// Create a new TDX provider using the default TSM path.
    pub fn new() -> Result<Self, TeeError> {
        Self::with_path(TSM_REPORT_PATH)
    }

    /// Create a new TDX provider with a custom TSM path.
    /// Primarily used for testing.
    pub fn with_path<P: AsRef<Path>>(path: P) -> Result<Self, TeeError> {
        let base_path = path.as_ref().to_path_buf();

        // Verify the TSM interface is available
        if !base_path.exists() {
            return Err(TeeError::NotSupported(format!(
                "TDX ConfigFS-TSM interface not found at {}. Ensure you are running in a TDX VM \
                 with kernel support.",
                base_path.display()
            )));
        }

        info!(target: "tee::tdx", path = %base_path.display(), "TDX provider initialized");

        Ok(Self { base_path })
    }

    /// Create a new report entry and return its path.
    fn create_report_entry(&self) -> Result<PathBuf, TeeError> {
        // Generate a unique name for this report request
        let entry_name = format!("katana_{}", Uuid::new_v4());
        let entry_path = self.base_path.join(&entry_name);

        // Create the report directory entry
        fs::create_dir(&entry_path).map_err(|e| {
            TeeError::GenerationFailed(format!(
                "Failed to create TSM report entry at {}: {}",
                entry_path.display(),
                e
            ))
        })?;

        debug!(target: "tee::tdx", path = %entry_path.display(), "Created TSM report entry");

        Ok(entry_path)
    }

    /// Clean up a report entry.
    fn cleanup_report_entry(&self, entry_path: &Path) {
        if let Err(e) = fs::remove_dir_all(entry_path) {
            warn!(
                target: "tee::tdx",
                path = %entry_path.display(),
                error = %e,
                "Failed to cleanup TSM report entry"
            );
        }
    }
}

impl TeeProvider for TdxProvider {
    fn generate_quote(&self, user_data: &[u8; 64]) -> Result<Vec<u8>, TeeError> {
        debug!(target: "tee::tdx", "Generating TDX attestation quote");

        let entry_path = self.create_report_entry()?;

        // Ensure cleanup happens even on error
        let result = (|| {
            // Write the user data (report_data) to the inblob file
            let inblob_path = entry_path.join("inblob");
            let mut inblob_file = File::create(&inblob_path)?;
            inblob_file.write_all(user_data)?;
            inblob_file.sync_all()?;

            debug!(target: "tee::tdx", path = %inblob_path.display(), "Wrote report data");

            // Read the generated quote from outblob
            let outblob_path = entry_path.join("outblob");
            let mut quote = Vec::new();
            let mut outblob_file = File::open(&outblob_path)?;
            outblob_file.read_to_end(&mut quote)?;

            debug!(
                target: "tee::tdx",
                quote_size = quote.len(),
                "Successfully generated TDX quote"
            );

            Ok(quote)
        })();

        self.cleanup_report_entry(&entry_path);

        result.map_err(|e: std::io::Error| {
            TeeError::GenerationFailed(format!("TDX quote generation failed: {}", e))
        })
    }

    fn provider_type(&self) -> &'static str {
        "TDX"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tdx_not_available() {
        let result = TdxProvider::with_path("/nonexistent/path");
        assert!(matches!(result, Err(TeeError::NotSupported(_))));
    }
}
