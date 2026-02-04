//! QEMU VM management for Katana TEE.

use crate::config::Config;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// VM start request parameters
#[derive(Debug, Clone, serde::Deserialize)]
pub struct StartRequest {
    /// Block number to fork from
    pub fork_block: u64,
    /// RPC URL of the chain to fork
    pub fork_provider: String,
    /// Katana RPC port inside VM (default: 5050)
    #[serde(default = "default_katana_port")]
    pub port: u16,
}

fn default_katana_port() -> u16 {
    5050
}

/// Current VM state
#[derive(Debug, Clone, serde::Serialize)]
pub struct VmStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub fork_block: Option<u64>,
    pub fork_provider: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub uptime_secs: Option<u64>,
}

/// Internal state for a running VM
struct RunningVm {
    process: Child,
    fork_block: u64,
    fork_provider: String,
    started_at: DateTime<Utc>,
    serial_log_path: PathBuf,
}

/// VM Manager handles QEMU process lifecycle
pub struct VmManager {
    config: Config,
    running_vm: Arc<Mutex<Option<RunningVm>>>,
}

impl VmManager {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            running_vm: Arc::new(Mutex::new(None)),
        }
    }

    /// Start a new VM with the given parameters
    pub async fn start(&self, req: StartRequest) -> Result<String> {
        let mut vm_guard = self.running_vm.lock().await;

        // Check if VM is already running
        if let Some(ref mut vm) = *vm_guard {
            if is_process_running(&mut vm.process) {
                anyhow::bail!(
                    "VM is already running (PID {}). Stop it first.",
                    vm.process.id()
                );
            }
        }

        // Validate boot components
        if !self.config.dry_run {
            self.config.validate_boot_components()?;
        }

        // Build kernel cmdline with Katana args
        let kernel_cmdline = format!(
            "console=ttyS0 katana.args=--http.addr,0.0.0.0,--http.port,{},--tee.provider,sev-snp,--fork.block,{},--fork.provider,{}",
            req.port, req.fork_block, req.fork_provider
        );

        info!("Starting VM with kernel cmdline: {}", kernel_cmdline);

        // Create temp file for serial log
        let serial_log_path = std::env::temp_dir().join(format!(
            "katana-tee-vm-serial-{}.log",
            std::process::id()
        ));

        let rpc_url = format!("http://localhost:{}", self.config.host_rpc_port);

        if self.config.dry_run {
            info!("DRY-RUN: Would start QEMU with:");
            info!("  OVMF: {:?}", self.config.ovmf_path());
            info!("  Kernel: {:?}", self.config.kernel_path());
            info!("  Initrd: {:?}", self.config.initrd_path());
            info!("  Cmdline: {}", kernel_cmdline);
            info!("  RPC URL: {}", rpc_url);

            // Create a fake "running" state for dry-run
            *vm_guard = Some(RunningVm {
                process: Command::new("sleep")
                    .arg("infinity")
                    .spawn()
                    .context("spawn sleep for dry-run")?,
                fork_block: req.fork_block,
                fork_provider: req.fork_provider,
                started_at: Utc::now(),
                serial_log_path,
            });

            return Ok(rpc_url);
        }

        // Build QEMU command
        // Based on start-vm.sh script
        let child = Command::new(&self.config.qemu_path)
            .arg("-enable-kvm")
            .arg("-cpu")
            .arg("EPYC-v4")
            .arg("-smp")
            .arg(self.config.vm_vcpus.to_string())
            .arg("-m")
            .arg(&self.config.vm_memory)
            .arg("-machine")
            .arg("q35,confidential-guest-support=sev0,vmport=off")
            .arg("-object")
            .arg(format!(
                "memory-backend-memfd,id=ram1,size={},share=true,prealloc=false",
                self.config.vm_memory
            ))
            .arg("-machine")
            .arg("memory-backend=ram1")
            .arg("-object")
            .arg("sev-snp-guest,id=sev0,policy=0x30000,cbitpos=51,reduced-phys-bits=1,kernel-hashes=on")
            .arg("-nographic")
            .arg("-serial")
            .arg(format!("file:{}", serial_log_path.display()))
            .arg("-bios")
            .arg(self.config.ovmf_path())
            .arg("-kernel")
            .arg(self.config.kernel_path())
            .arg("-initrd")
            .arg(self.config.initrd_path())
            .arg("-append")
            .arg(&kernel_cmdline)
            .arg("-netdev")
            .arg(format!(
                "user,id=net0,hostfwd=tcp::{}-:{}",
                self.config.host_rpc_port, req.port
            ))
            .arg("-device")
            .arg("virtio-net-pci,disable-legacy=on,iommu_platform=true,netdev=net0,romfile=")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn QEMU process")?;

        let pid = child.id();
        info!("QEMU started with PID {}", pid);

        *vm_guard = Some(RunningVm {
            process: child,
            fork_block: req.fork_block,
            fork_provider: req.fork_provider,
            started_at: Utc::now(),
            serial_log_path,
        });

        Ok(rpc_url)
    }

    /// Stop the running VM
    pub async fn stop(&self) -> Result<()> {
        let mut vm_guard = self.running_vm.lock().await;

        if let Some(mut vm) = vm_guard.take() {
            let pid = vm.process.id();
            info!("Stopping VM (PID {})", pid);

            // Try graceful kill first
            if let Err(e) = vm.process.kill() {
                warn!("Failed to kill process: {}", e);
            }

            // Wait for process to exit
            match vm.process.wait() {
                Ok(status) => info!("VM exited with status: {}", status),
                Err(e) => warn!("Failed to wait for process: {}", e),
            }

            // Clean up serial log
            if vm.serial_log_path.exists() {
                if let Err(e) = std::fs::remove_file(&vm.serial_log_path) {
                    warn!("Failed to remove serial log: {}", e);
                }
            }

            Ok(())
        } else {
            anyhow::bail!("No VM is currently running")
        }
    }

    /// Get current VM status
    pub async fn status(&self) -> VmStatus {
        let mut vm_guard = self.running_vm.lock().await;

        if let Some(ref mut vm) = *vm_guard {
            let running = is_process_running(&mut vm.process);
            let uptime = if running {
                Some((Utc::now() - vm.started_at).num_seconds() as u64)
            } else {
                None
            };

            VmStatus {
                running,
                pid: if running { Some(vm.process.id()) } else { None },
                fork_block: Some(vm.fork_block),
                fork_provider: Some(vm.fork_provider.clone()),
                started_at: Some(vm.started_at),
                uptime_secs: uptime,
            }
        } else {
            VmStatus {
                running: false,
                pid: None,
                fork_block: None,
                fork_provider: None,
                started_at: None,
                uptime_secs: None,
            }
        }
    }

    /// Get serial console logs
    pub async fn logs(&self, lines: usize) -> Result<String> {
        let vm_guard = self.running_vm.lock().await;

        if let Some(ref vm) = *vm_guard {
            if vm.serial_log_path.exists() {
                let content = tokio::fs::read_to_string(&vm.serial_log_path)
                    .await
                    .context("Failed to read serial log")?;

                // Return last N lines
                let all_lines: Vec<&str> = content.lines().collect();
                let start = all_lines.len().saturating_sub(lines);
                Ok(all_lines[start..].join("\n"))
            } else {
                Ok(String::new())
            }
        } else {
            anyhow::bail!("No VM is currently running")
        }
    }
}

/// Check if a process is still running
fn is_process_running(child: &mut Child) -> bool {
    match child.try_wait() {
        Ok(Some(_)) => false, // Process has exited
        Ok(None) => true,     // Process is still running
        Err(_) => false,      // Error checking, assume not running
    }
}
