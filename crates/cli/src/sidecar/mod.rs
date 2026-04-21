//! Sidecar binary management for Katana.
//!
//! Katana optionally runs two external services as sidecar processes:
//!
//! - **`paymaster-service`** — an Avnu paymaster that sponsors user transactions.
//! - **`vrf-server`** — a VRF proof generator for verifiable randomness.
//!
//! These binaries are built from separate repositories
//! ([cartridge-gg/paymaster](https://github.com/cartridge-gg/paymaster) and
//! [cartridge-gg/vrf](https://github.com/cartridge-gg/vrf)) and are pinned to
//! specific git revisions in `sidecar-versions.toml` at the repository root.
//! Pre-built binaries are published as assets on each Katana GitHub release.
//!
//! # Binary resolution
//!
//! When Katana starts with `--paymaster` or `--vrf` in **sidecar mode** (the
//! default — no `--paymaster.url` / `--vrf.url` provided), it needs to locate
//! the sidecar binary on disk. The resolution order is:
//!
//! 1. **Explicit path** — `--paymaster.bin <PATH>` or `--vrf.bin <PATH>`. If provided, the file
//!    must exist or startup fails immediately.
//!
//! 2. **`PATH` lookup** — searches in `$PATH` for a file named `paymaster-service` (or
//!    `vrf-server`).
//!
//! 3. **`~/.katana/bin/`** — the managed install directory. If the binary exists here it is used
//!    directly.
//!
//! 4. **Lazy download** — if no binary is found anywhere, the user is prompted to download the
//!    prebuilt binary from the matching Katana GitHub release. See [Download and
//!    install](#download-and-install) below.
//!
//! No version checking is performed at any step. The sidecar binaries have their
//! own independent version schemes that do not correspond to Katana's release
//! tags, and `paymaster-service` doesn't even support `--version`.
//!
//! # Lazy download
//!
//! When no binary is found, the user is prompted to download a prebuilt binary
//! from the matching Katana GitHub release. This requires an interactive
//! terminal (TTY) — in non-interactive environments the process fails with
//! manual installation instructions instead. The downloaded archive is verified
//! against `checksums.txt` (SHA-256) before extraction and installation to
//! `~/.katana/bin/`.
//!
//! # Assumptions
//!
//! - Sidecar binaries are bundled as release artifacts on Katana's GitHub release. The artifact
//!   names are tagged with Katana's version (e.g. `paymaster-service_v1.7.0_darwin_arm64.tar.gz`),
//!   but this does **not** reflect the actual version of the sidecar — their versions are managed
//!   independently by their respective repositories. The download URL is derived from
//!   `CARGO_PKG_VERSION` at compile time.
//! - CI generates a `checksums.txt` in each release containing `sha256sum` output for every
//!   artifact. The filename is matched exactly (no directory prefix).
//! - Archives are `.tar.gz` on Linux/macOS and `.zip` on Windows, each containing the bare binary
//!   at the archive root.

mod github;
mod platform;
mod verify;

use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
pub use cartridge::vrf::server::{
    get_default_vrf_account, VrfAccountCredentials, VrfBootstrapResult, VrfServer, VrfServerConfig,
    VrfServiceProcess,
};
use katana_chain_spec::ChainSpec;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
pub use katana_paymaster::{
    format_felt, wait_for_paymaster_ready, PaymasterService, PaymasterServiceConfig,
    PaymasterServiceConfigBuilder, PaymasterSidecarProcess,
};
use katana_primitives::{ContractAddress, Felt};
pub use platform::Platform;
use tracing::debug;
use url::Url;

/// Default API key for the paymaster sidecar.
pub const DEFAULT_PAYMASTER_API_KEY: &str = "paymaster_katana";

pub async fn bootstrap_paymaster(
    bin_path: PathBuf,
    paymaster_url: Url,
    rpc_url: SocketAddr,
    chain: &ChainSpec,
) -> Result<PaymasterService> {
    let (relayer_addr, relayer_pk) = prefunded_account(chain, 0)?;
    let (gas_tank_addr, gas_tank_pk) = prefunded_account(chain, 1)?;
    let (estimate_account_addr, estimate_account_pk) = prefunded_account(chain, 2)?;

    let port = paymaster_url.port().unwrap();

    let builder = PaymasterServiceConfigBuilder::new()
        .rpc(rpc_url)
        .port(port)
        .api_key(DEFAULT_PAYMASTER_API_KEY)
        .relayer(relayer_addr, relayer_pk)
        .gas_tank(gas_tank_addr, gas_tank_pk)
        .estimate_account(estimate_account_addr, estimate_account_pk)
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .program_path(bin_path);

    let mut paymaster = PaymasterService::new(builder.build().await?).chain_id(chain.id());
    paymaster.bootstrap().await?;

    Ok(paymaster)
}

pub async fn bootstrap_vrf(
    bin_path: PathBuf,
    vrf_url: Url,
    rpc_addr: SocketAddr,
    chain: &ChainSpec,
) -> Result<VrfServer> {
    let rpc_url = local_rpc_url(&rpc_addr);
    let (account_address, pk) = prefunded_account(chain, 0)?;

    let result = cartridge::vrf::server::bootstrap_vrf(rpc_url, account_address, pk).await?;

    let vrf_service = VrfServer::new(VrfServerConfig {
        port: vrf_url.port().unwrap(),
        secret_key: result.secret_key,
        vrf_account_address: result.vrf_account_address,
        vrf_private_key: result.vrf_account_private_key,
    })
    .path(bin_path);

    Ok(vrf_service)
}

fn prefunded_account(chain_spec: &ChainSpec, index: u16) -> Result<(ContractAddress, Felt)> {
    let (address, allocation) = chain_spec
        .genesis()
        .accounts()
        .nth(index as usize)
        .ok_or_else(|| anyhow!("prefunded account index {} out of range", index))?;

    let private_key = match allocation {
        GenesisAccountAlloc::DevAccount(account) => account.private_key,
        _ => return Err(anyhow!("prefunded account {} has no private key", address)),
    };

    Ok((*address, private_key))
}

fn local_rpc_url(addr: &SocketAddr) -> Url {
    let host = match addr.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4([127, 0, 0, 1].into())
        }
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4([127, 0, 0, 1].into())
        }
        ip => ip,
    };

    Url::parse(&format!("http://{}:{}", host, addr.port())).expect("valid rpc url")
}

/// Known sidecar binaries that katana can manage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarKind {
    Paymaster,
    Vrf,
}

impl SidecarKind {
    /// The binary name for this sidecar.
    pub const fn binary_name(&self) -> &'static str {
        match self {
            Self::Paymaster => "paymaster-service",
            Self::Vrf => "vrf-server",
        }
    }

    /// The binary name with platform extension (e.g., .exe on Windows).
    pub const fn binary_filename(&self) -> &'static str {
        #[cfg(windows)]
        match self {
            Self::Paymaster => "paymaster-service.exe",
            Self::Vrf => "vrf-server.exe",
        }

        #[cfg(not(windows))]
        self.binary_name()
    }

    /// The release artifact name for a given version and platform.
    ///
    /// e.g., `paymaster-service_v1.2.3_linux_amd64.tar.gz`
    pub fn artifact_name(&self, version: &str, platform: &Platform) -> String {
        let ext = platform.archive_extension();
        format!("{}_{}_{}_{}.{}", self.binary_name(), version, platform.os, platform.arch, ext)
    }
}

impl std::fmt::Display for SidecarKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.binary_name())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("sidecar binary not found at explicit path: {0}")]
    ExplicitPathNotFound(PathBuf),

    #[error("failed to read user input: {0}")]
    ReadInput(#[source] std::io::Error),

    #[error(
        "cannot prompt for installation: not running in an interactive terminal.\nInstall \
         manually: download {binary} from the GitHub release and place it in {path}"
    )]
    NotInteractive { binary: String, path: String },

    #[error("failed to install sidecar binary from github release: {0}")]
    GithubReleaseInstall(#[from] github::GithubReleaseInstallError),
}

/// Resolve or install a sidecar binary.
///
/// Resolution order:-
/// 1. Explicit path (if provided via CLI flag)
/// 2. Search PATH
/// 3. Search ~/.katana/bin/
/// 4. Prompt user and download from GitHub release
pub async fn resolve_sidecar_binary(
    kind: SidecarKind,
    explicit_path: Option<&Path>,
) -> Result<Option<PathBuf>, ResolveError> {
    // 1. Explicit path from CLI
    if let Some(path) = explicit_path {
        debug!(sidecar = %kind, path = %path.display(), "Using explicitly provided sidecar binary.");
        return Ok(Some(path.to_path_buf()));
    }

    let binary_name = kind.binary_filename();
    let bin_dir = sidecar_bin_dir();

    // 2. Search in $PATH
    if let Some(path) = search_in_PATH(binary_name) {
        debug!(sidecar = %kind, path = %path.display(), "Found sidecar binary in $PATH.");
        return Ok(Some(path));
    }

    // 3. Search ~/.katana/bin/
    let home_path = bin_dir.join(binary_name);
    if home_path.is_file() {
        debug!(sidecar = %kind, path = %home_path.display(), "Found sidecar binary in ~/.katana/bin/ .");
        return Ok(Some(home_path));
    }

    // 4. Download from GitHub release
    if prompt_download(kind)? {
        Ok(Some(github::install(kind).await?))
    } else {
        eprintln!("user declined installation");
        Ok(None)
    }
}

/// The expected sidecar version tag for this build of katana.
///
/// Sidecar binaries are released as assets on katana's GitHub release,
/// so the expected version matches katana's own version.
fn current_version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// Search for a binary in the PATH environment variable.
#[allow(non_snake_case)]
fn search_in_PATH(binary_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The default base directory for katana data (~/.katana).
fn katana_home() -> PathBuf {
    dirs::home_dir().expect("failed to determine home directory").join(".katana")
}

/// The directory where sidecar binaries are installed (~/.katana/bin).
fn sidecar_bin_dir() -> PathBuf {
    katana_home().join("bin")
}

/// Prompt the user to download a sidecar binary.
fn prompt_download(kind: SidecarKind) -> Result<bool, ResolveError> {
    /// Read a yes/no (y/n) response from stdin.
    fn read_yes_no() -> Result<bool, ResolveError> {
        let mut input = String::new();
        std::io::stdin().lock().read_line(&mut input).map_err(ResolveError::ReadInput)?;
        let input = input.trim().to_lowercase();
        Ok(input == "y" || input == "yes")
    }

    let binary = kind.binary_name();

    ensure_interactive(kind)?;
    eprint!("{binary} not found. Download {binary} for your platform? [y/N] ");

    std::io::stderr().flush().ok();
    read_yes_no()
}

/// Ensure we're running in an interactive terminal.
fn ensure_interactive(kind: SidecarKind) -> Result<(), ResolveError> {
    if !atty::is(atty::Stream::Stdin) {
        return Err(ResolveError::NotInteractive {
            binary: kind.binary_name().to_string(),
            path: sidecar_bin_dir().display().to_string(),
        });
    }

    Ok(())
}
