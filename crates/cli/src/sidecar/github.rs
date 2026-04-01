use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::debug;

use super::platform::Platform;
use super::verify::{self, VerifyError};
use super::SidecarKind;
use crate::sidecar::platform::detect_platform;
use crate::sidecar::{current_version, sidecar_bin_dir};

/// GitHub repository for release artifact downloads.
const GITHUB_REPO: &str = "dojoengine/katana";

#[derive(Debug, Error)]
pub enum GithubReleaseInstallError {
    #[error(transparent)]
    Download(#[from] DownloadError),

    #[error("failed to create sidecar bin directory {path}: {source}")]
    CreateDir { path: PathBuf, source: std::io::Error },

    #[error("failed to copy binary to {dest}: {source}")]
    Copy { dest: PathBuf, source: std::io::Error },

    #[error("failed to set executable permissions: {0}")]
    SetPermissions(#[source] std::io::Error),
}

/// Install a sidecar binary from GitHub release.
pub async fn install(kind: SidecarKind) -> Result<PathBuf, GithubReleaseInstallError> {
    eprintln!("{kind} not found locally, attempting download...");

    let platform = detect_platform();
    let version = current_version();
    let bin_dir = sidecar_bin_dir();

    let downloaded = download_sidecar(kind, &version, &platform).await?;
    let installed = install_sidecar(kind, &downloaded, &bin_dir)?;

    Ok(installed)
}

/// Install a sidecar binary to the target directory (~/.katana/bin/).
///
/// - Copies the binary from `source` to `<target_dir>/<binary_name>`
/// - Sets executable permissions (Unix)
///
/// Returns the path to the installed binary.
fn install_sidecar(
    kind: SidecarKind,
    source: &Path,
    target_dir: &Path,
) -> Result<PathBuf, GithubReleaseInstallError> {
    // Ensure the target directory exists
    fs::create_dir_all(target_dir).map_err(|source| GithubReleaseInstallError::CreateDir {
        path: target_dir.to_path_buf(),
        source,
    })?;

    let binary_filename = kind.binary_filename();
    let dest = target_dir.join(binary_filename);

    // Copy the binary
    fs::copy(source, &dest)
        .map_err(|source| GithubReleaseInstallError::Copy { dest: dest.clone(), source })?;

    // Set executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&dest, perms).map_err(GithubReleaseInstallError::SetPermissions)?;
    }

    eprintln!("installed {binary_filename} binary at {}", dest.display());

    Ok(dest)
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("failed to download {url}: {source}")]
    Request { url: String, source: reqwest::Error },

    #[error("download failed with status {status} for {url}")]
    HttpStatus { url: String, status: reqwest::StatusCode },

    #[error("failed to read response body: {0}")]
    ReadBody(#[source] reqwest::Error),

    #[error("failed to create temporary directory: {0}")]
    TempDir(#[source] std::io::Error),

    #[error("failed to write archive to disk: {0}")]
    WriteArchive(#[source] std::io::Error),

    #[error("checksum verification failed")]
    Checksum(#[from] VerifyError),

    #[error("failed to extract archive: {0}")]
    Extract(String),
}

/// Download a sidecar binary from GitHub releases, verify its checksum, and return the
/// path to the extracted binary in a temporary directory.
///
/// The caller is responsible for moving the binary to its final destination.
async fn download_sidecar(
    kind: SidecarKind,
    version: &str,
    platform: &Platform,
) -> Result<PathBuf, DownloadError> {
    let artifact_name = kind.artifact_name(version, platform);
    let url = release_artifact_url(version, &artifact_name);
    let checksums_url = checksums_url(version);

    debug!(%url, "downloading sidecar binary");

    let client = reqwest::Client::new();

    // Download checksums.txt
    let checksums_resp = client
        .get(&checksums_url)
        .send()
        .await
        .map_err(|e| DownloadError::Request { url: checksums_url.clone(), source: e })?;

    if !checksums_resp.status().is_success() {
        return Err(DownloadError::HttpStatus {
            url: checksums_url,
            status: checksums_resp.status(),
        });
    }

    let checksums_content = checksums_resp
        .text()
        .await
        .map_err(VerifyError::ReadChecksums)
        .map_err(DownloadError::Checksum)?;

    // Download the archive
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| DownloadError::Request { url: url.clone(), source: e })?;

    if !resp.status().is_success() {
        return Err(DownloadError::HttpStatus { url, status: resp.status() });
    }

    let archive_bytes = resp.bytes().await.map_err(DownloadError::ReadBody)?;

    // Write archive to temp dir
    let tmp_dir = tempfile::tempdir().map_err(DownloadError::TempDir)?;
    let archive_path = tmp_dir.path().join(&artifact_name);
    std::fs::write(&archive_path, &archive_bytes).map_err(DownloadError::WriteArchive)?;

    // Verify checksum
    verify::verify_checksum(&archive_path, &artifact_name, &checksums_content)?;

    debug!("checksum verified, extracting archive");

    // Extract the binary
    let binary_path = extract_binary(&archive_path, kind, platform, tmp_dir.path())?;

    // Keep the temp dir alive by leaking it — the caller will move the binary out
    let _ = tmp_dir.keep();

    Ok(binary_path)
}

/// Construct the download URL for a sidecar release artifact.
fn release_artifact_url(version: &str, artifact_name: &str) -> String {
    format!("https://github.com/{GITHUB_REPO}/releases/download/{version}/{artifact_name}")
}

/// Construct the download URL for the checksums file.
fn checksums_url(version: &str) -> String {
    format!("https://github.com/{GITHUB_REPO}/releases/download/{version}/checksums.txt")
}

/// Extract the sidecar binary from an archive.
fn extract_binary(
    archive_path: &Path,
    kind: SidecarKind,
    platform: &Platform,
    dest_dir: &Path,
) -> Result<PathBuf, DownloadError> {
    let binary_filename = kind.binary_filename();

    if platform.archive_extension() == "zip" {
        extract_zip(archive_path, binary_filename, dest_dir)
    } else {
        extract_tar_gz(archive_path, binary_filename, dest_dir)
    }
}

fn extract_tar_gz(
    archive_path: &Path,
    binary_filename: &str,
    dest_dir: &Path,
) -> Result<PathBuf, DownloadError> {
    let file =
        std::fs::File::open(archive_path).map_err(|e| DownloadError::Extract(e.to_string()))?;
    let decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().map_err(|e| DownloadError::Extract(e.to_string()))? {
        let mut entry = entry.map_err(|e| DownloadError::Extract(e.to_string()))?;
        let path = entry.path().map_err(|e| DownloadError::Extract(e.to_string()))?;

        // Match the binary by filename (it may be at root or in a subdirectory)
        if path.file_name().and_then(|n| n.to_str()) == Some(binary_filename) {
            let dest = dest_dir.join(binary_filename);
            let mut out =
                std::fs::File::create(&dest).map_err(|e| DownloadError::Extract(e.to_string()))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| DownloadError::Extract(e.to_string()))?;
            return Ok(dest);
        }
    }

    Err(DownloadError::Extract(format!("binary '{binary_filename}' not found in archive")))
}

fn extract_zip(
    archive_path: &Path,
    binary_filename: &str,
    dest_dir: &Path,
) -> Result<PathBuf, DownloadError> {
    let file =
        std::fs::File::open(archive_path).map_err(|e| DownloadError::Extract(e.to_string()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| DownloadError::Extract(e.to_string()))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| DownloadError::Extract(e.to_string()))?;
        let name = entry.name().to_string();

        if name.ends_with(binary_filename) {
            let dest = dest_dir.join(binary_filename);
            let mut out =
                std::fs::File::create(&dest).map_err(|e| DownloadError::Extract(e.to_string()))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| DownloadError::Extract(e.to_string()))?;
            return Ok(dest);
        }
    }

    Err(DownloadError::Extract(format!("binary '{binary_filename}' not found in archive")))
}
