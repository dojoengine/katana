use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::info;

use super::SidecarKind;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("failed to create sidecar bin directory {path}: {source}")]
    CreateDir { path: PathBuf, source: std::io::Error },

    #[error("failed to copy binary to {dest}: {source}")]
    Copy { dest: PathBuf, source: std::io::Error },

    #[error("failed to set executable permissions: {0}")]
    SetPermissions(#[source] std::io::Error),
}

/// Install a sidecar binary to the target directory (~/.katana/bin/).
///
/// - Copies the binary from `source` to `<target_dir>/<binary_name>`
/// - Sets executable permissions (Unix)
///
/// Returns the path to the installed binary.
pub fn install_sidecar(
    kind: SidecarKind,
    source: &Path,
    target_dir: &Path,
) -> Result<PathBuf, InstallError> {
    // Ensure the target directory exists
    std::fs::create_dir_all(target_dir)
        .map_err(|e| InstallError::CreateDir { path: target_dir.to_path_buf(), source: e })?;

    let binary_filename = kind.binary_filename();
    let dest = target_dir.join(binary_filename);

    // Copy the binary
    std::fs::copy(source, &dest)
        .map_err(|e| InstallError::Copy { dest: dest.clone(), source: e })?;

    // Set executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&dest, perms).map_err(InstallError::SetPermissions)?;
    }

    info!(path = %dest.display(), "installed sidecar binary");

    Ok(dest)
}
