use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::{debug, info};

use super::download::{self, DownloadError};
use super::install::{self, InstallError};
use super::platform::detect_platform;
use super::SidecarKind;

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("sidecar binary not found at explicit path: {0}")]
    ExplicitPathNotFound(PathBuf),

    #[error("user declined to install {0}")]
    UserDeclined(SidecarKind),

    #[error(
        "cannot prompt for installation: not running in an interactive terminal.\nInstall \
         manually: download {binary} from the GitHub release and place it in {path}"
    )]
    NotInteractive { binary: String, path: String },

    #[error("failed to download sidecar binary")]
    Download(#[from] DownloadError),

    #[error("failed to install sidecar binary")]
    Install(#[from] InstallError),

    #[error("failed to read user input: {0}")]
    ReadInput(#[source] io::Error),
}

/// Information about a resolved sidecar binary.
#[derive(Debug)]
pub struct SidecarBinary {
    /// Path to the binary.
    pub path: PathBuf,
}

/// Resolve or install a sidecar binary.
///
/// Resolution order:
/// 1. Explicit path (if provided via CLI flag)
/// 2. Search PATH
/// 3. Search ~/.katana/bin/
/// 4. Prompt user and download from GitHub release
pub async fn resolve_or_install(
    kind: SidecarKind,
    explicit_path: Option<&Path>,
    expected_version: &str,
) -> Result<SidecarBinary, ResolveError> {
    // 1. Explicit path
    if let Some(path) = explicit_path {
        if path.is_file() {
            debug!(path = %path.display(), "using explicitly provided sidecar binary");
            return Ok(SidecarBinary { path: path.to_path_buf() });
        }
        return Err(ResolveError::ExplicitPathNotFound(path.to_path_buf()));
    }

    let binary_name = kind.binary_filename();
    let bin_dir = super::sidecar_bin_dir();

    // 2. Search PATH
    if let Some(path) = search_path(binary_name) {
        debug!(path = %path.display(), "found sidecar binary in PATH");
        return Ok(SidecarBinary { path });
    }

    // 3. Search ~/.katana/bin/
    let home_path = bin_dir.join(binary_name);
    if home_path.is_file() {
        debug!(path = %home_path.display(), "found sidecar binary in ~/.katana/bin/");
        return Ok(SidecarBinary { path: home_path });
    }

    // 4. Download from GitHub release
    info!("{kind} not found locally, attempting download");

    if !prompt_download(kind, expected_version)? {
        return Err(ResolveError::UserDeclined(kind));
    }

    let platform = detect_platform();
    let downloaded = download::download_sidecar(kind, expected_version, &platform).await?;
    let installed = install::install_sidecar(kind, &downloaded, &bin_dir)?;

    Ok(SidecarBinary { path: installed })
}

/// Search for a binary in the PATH environment variable.
fn search_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Prompt the user to download a sidecar binary.
fn prompt_download(kind: SidecarKind, version: &str) -> Result<bool, ResolveError> {
    ensure_interactive(kind)?;

    eprint!("{} not found. Download {} for your platform? [y/N] ", kind.binary_name(), version);
    io::stderr().flush().ok();

    read_yes_no()
}

/// Ensure we're running in an interactive terminal.
fn ensure_interactive(kind: SidecarKind) -> Result<(), ResolveError> {
    if !atty::is(atty::Stream::Stdin) {
        return Err(ResolveError::NotInteractive {
            binary: kind.binary_name().to_string(),
            path: super::sidecar_bin_dir().display().to_string(),
        });
    }
    Ok(())
}

/// Read a yes/no response from stdin.
fn read_yes_no() -> Result<bool, ResolveError> {
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input).map_err(ResolveError::ReadInput)?;
    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes")
}
