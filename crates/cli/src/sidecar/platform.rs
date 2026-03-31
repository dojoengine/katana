/// Represents the current platform for artifact naming.
#[derive(Debug, Clone)]
pub struct Platform {
    /// OS identifier used in release artifacts (e.g., "linux", "darwin", "win32").
    pub os: &'static str,
    /// Architecture identifier used in release artifacts (e.g., "amd64", "arm64").
    pub arch: &'static str,
}

impl Platform {
    /// The file extension for release archives on this platform.
    pub fn archive_extension(&self) -> &'static str {
        if self.os == "win32" {
            "zip"
        } else {
            "tar.gz"
        }
    }
}

/// Detect the current platform for artifact naming.
///
/// Maps `std::env::consts::{OS, ARCH}` to the naming convention used in katana releases.
pub fn detect_platform() -> Platform {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "win32",
        other => panic!("unsupported OS: {other}"),
    };

    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => panic!("unsupported architecture: {other}"),
    };

    Platform { os, arch }
}
