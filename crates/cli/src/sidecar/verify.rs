use std::path::Path;

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("failed to read file for checksum verification: {0}")]
    ReadFile(#[source] std::io::Error),

    #[error("failed to read checksums response: {0}")]
    ReadChecksums(#[source] reqwest::Error),

    #[error("no checksum found for artifact '{0}' in checksums.txt")]
    ArtifactNotFound(String),

    #[error("checksum mismatch for '{artifact}': expected {expected}, got {actual}")]
    Mismatch { artifact: String, expected: String, actual: String },
}

/// Verify the SHA256 checksum of a file against a checksums.txt content.
///
/// The checksums content is expected to be in the format:
/// ```text
/// <sha256hex>  <filename>
/// ```
pub fn verify_checksum(
    file_path: &Path,
    artifact_name: &str,
    checksums_content: &str,
) -> Result<(), VerifyError> {
    let file_bytes = std::fs::read(file_path).map_err(VerifyError::ReadFile)?;

    let mut hasher = Sha256::new();
    hasher.update(&file_bytes);
    let actual = format!("{:x}", hasher.finalize());

    let expected = checksums_content
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            if name == artifact_name {
                Some(hash.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| VerifyError::ArtifactNotFound(artifact_name.to_string()))?;

    if actual != expected {
        return Err(VerifyError::Mismatch {
            artifact: artifact_name.to_string(),
            expected,
            actual,
        });
    }

    Ok(())
}
