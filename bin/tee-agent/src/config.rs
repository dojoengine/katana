//! Configuration and CLI arguments for tee-agent.

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "tee-agent")]
#[command(about = "HTTP API agent for managing Katana TEE VM", long_about = None)]
pub struct Config {
    /// Path to boot components directory (containing OVMF.fd, vmlinuz, initrd.img)
    #[arg(long, env = "BOOT_DIR", default_value = "misc/AMDSEV/output/qemu")]
    pub boot_dir: PathBuf,

    /// HTTP server port
    #[arg(long, env = "AGENT_PORT", default_value = "8080")]
    pub port: u16,

    /// HTTP server host
    #[arg(long, env = "AGENT_HOST", default_value = "0.0.0.0")]
    pub host: String,

    /// Host port to forward Katana RPC (VM internal port -> this host port)
    #[arg(long, env = "HOST_RPC_PORT", default_value = "15051")]
    pub host_rpc_port: u16,

    /// VM memory size
    #[arg(long, env = "VM_MEMORY", default_value = "512M")]
    pub vm_memory: String,

    /// Number of vCPUs
    #[arg(long, env = "VM_VCPUS", default_value = "1")]
    pub vm_vcpus: u32,

    /// Dry-run mode (don't actually start QEMU, for testing)
    #[arg(long)]
    pub dry_run: bool,

    /// Path to qemu-system-x86_64 binary
    #[arg(long, env = "QEMU_PATH", default_value = "qemu-system-x86_64")]
    pub qemu_path: String,
}

impl Config {
    /// Get path to OVMF firmware file
    pub fn ovmf_path(&self) -> PathBuf {
        self.boot_dir.join("OVMF.fd")
    }

    /// Get path to kernel file
    pub fn kernel_path(&self) -> PathBuf {
        self.boot_dir.join("vmlinuz")
    }

    /// Get path to initrd file
    pub fn initrd_path(&self) -> PathBuf {
        self.boot_dir.join("initrd.img")
    }

    /// Validate that all required boot components exist
    pub fn validate_boot_components(&self) -> anyhow::Result<()> {
        let components = [
            ("OVMF.fd", self.ovmf_path()),
            ("vmlinuz", self.kernel_path()),
            ("initrd.img", self.initrd_path()),
        ];

        for (name, path) in components {
            if !path.exists() {
                anyhow::bail!(
                    "Missing boot component '{}' at {:?}. \
                     Make sure --boot-dir points to a directory with OVMF.fd, vmlinuz, and initrd.img",
                    name,
                    path
                );
            }
        }

        Ok(())
    }
}
