//! L2 contract deployment via `saya-ops`.
//!
//! Shells out to the `saya-ops` binary (built from
//! `dojoengine/saya@5a3b8c9`) to declare and deploy:
//!
//! 1. The `mock_amd_tee_registry` contract — a permissive `IAMDTeeRegistry` mock from
//!    `cartridge-gg/piltover` (added in piltover#15), vendored into saya at
//!    `contracts/tee_registry_mock.json` and embedded in the `saya-ops` binary.
//! 2. The Piltover core contract.
//! 3. `setup-program` against Piltover, pointing the `fact_registry_address` at the deployed mock
//!    TEE registry so its on-chain `verify_sp1_proof` becomes a passthrough.
//!
//! `saya-ops` is resolved via `SAYA_OPS_BIN` env var or `$PATH`. Address
//! parsing scrapes the `info!`-logged "X address: Felt(0x…)" lines from
//! stdout/stderr (saya-ops uses `env_logger` which writes to stderr by
//! default; we capture both).

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use starknet_types_core::felt::Felt;

use crate::nodes::L2InProcess;

const CHAIN_ID_SHORT_STRING: &str = "katana_e2e";
const FACT_REGISTRY_SALT: &str = "0x53fac7";
const PILTOVER_SALT: &str = "0x9117f0";
const TEE_REGISTRY_SALT: &str = "0x7ee";

/// Runs the full L2 bootstrap sequence.
pub async fn bootstrap_l2(l2: &L2InProcess) -> Result<BootstrapResult> {
    let l2_url = l2.url();

    // Pull the prefunded account from the dev genesis.
    let (account_address, account_private_key) = l2.prefunded_account_keys();

    let saya = SayaOps {
        rpc_url: l2_url.to_string(),
        account_address: account_address.into(),
        account_private_key,
        chain_id: "SN_SEPOLIA".to_string(),
    };

    println!("Declaring + deploying mock TEE registry on L2");
    let tee_registry_address = saya.declare_and_deploy_tee_registry_mock()?;
    println!("  tee_registry_address={}", hex(&tee_registry_address));

    println!("Declaring Piltover core contract on L2");
    saya.declare_core_contract()?;

    println!("Deploying Piltover core contract on L2");
    let piltover_address = saya.deploy_core_contract()?;
    println!("  piltover_address={}", hex(&piltover_address));

    println!("Configuring Piltover with mock TEE registry as fact_registry_address");
    saya.setup_program(piltover_address, tee_registry_address)?;

    Ok(BootstrapResult {
        piltover_address,
        tee_registry_address,
        account_address: account_address.into(),
        account_private_key,
    })
}

#[derive(Debug, Clone)]
pub struct BootstrapResult {
    /// The address of the deployed Piltover core contract on the settlement layer.
    pub piltover_address: Felt,
    /// The address of the deployed TEE regsitry contract on the settlement layer.
    pub tee_registry_address: Felt,
    /// The address of the account used for the bootstrapping.
    pub account_address: Felt,
    /// The private key of the account used for the bootstrapping.
    pub account_private_key: Felt,
}

#[derive(Debug, Clone)]
struct SayaOps {
    rpc_url: String,
    account_address: Felt,
    account_private_key: Felt,
    chain_id: String,
}

impl SayaOps {
    fn declare_and_deploy_tee_registry_mock(&self) -> Result<Felt> {
        let mut cmd = self.base_command()?;
        cmd.args([
            "core-contract",
            "declare-and-deploy-tee-registry-mock",
            "--salt",
            TEE_REGISTRY_SALT,
        ]);

        let output = run(cmd, "declare-and-deploy-tee-registry-mock")?;
        parse_address("TEE registry mock address", &output)
    }

    fn declare_core_contract(&self) -> Result<()> {
        let mut cmd = self.base_command()?;
        cmd.args(["core-contract", "declare"]);
        run(cmd, "core-contract declare")?;
        Ok(())
    }

    fn deploy_core_contract(&self) -> Result<Felt> {
        let mut cmd = self.base_command()?;
        cmd.args(["core-contract", "deploy", "--salt", PILTOVER_SALT]);
        let output = run(cmd, "core-contract deploy")?;
        parse_address("Core contract address", &output)
    }

    fn setup_program(&self, core_contract: Felt, fact_registry: Felt) -> Result<()> {
        let mut cmd = self.base_command()?;
        cmd.args([
            "core-contract",
            "setup-program",
            "--core-contract-address",
            &hex(&core_contract),
            "--fact-registry-address",
            &hex(&fact_registry),
            "--chain-id",
            CHAIN_ID_SHORT_STRING,
        ]);
        let _ = FACT_REGISTRY_SALT; // currently unused — left for future fact-registry-mock variant
        run(cmd, "core-contract setup-program")?;
        Ok(())
    }

    fn base_command(&self) -> Result<Command> {
        let bin = resolve_saya_ops_bin()?;
        let mut cmd = Command::new(bin);
        cmd.env("SETTLEMENT_RPC_URL", &self.rpc_url)
            .env("SETTLEMENT_ACCOUNT_ADDRESS", hex(&self.account_address))
            .env("SETTLEMENT_ACCOUNT_PRIVATE_KEY", hex(&self.account_private_key))
            .env("SETTLEMENT_CHAIN_ID", &self.chain_id)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(cmd)
    }
}

/// Captured output of a saya-ops invocation.
struct CapturedOutput {
    combined: String,
}

fn run(mut cmd: Command, label: &str) -> Result<CapturedOutput> {
    eprintln!("[debug] running saya-ops: {cmd:?}");
    let output = cmd.output().with_context(|| format!("failed to spawn `saya-ops` for {label}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{stdout}\n{stderr}");

    if !output.status.success() {
        return Err(anyhow!(
            "saya-ops `{label}` exited with {}:\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        ));
    }

    Ok(CapturedOutput { combined })
}

/// Scans saya-ops output for an `info!` log line of the form:
///
///   `X address: Felt(0x…)` or `X address: 0x…`
///
/// where `X` is the supplied label (e.g. `"TEE registry mock address"`).
/// `info!` writes via env_logger to stderr, hence we search the combined
/// stdout+stderr buffer.
fn parse_address(label: &str, output: &CapturedOutput) -> Result<Felt> {
    for line in output.combined.lines() {
        let Some(idx) = line.find(label) else { continue };
        let rest = &line[idx + label.len()..];
        // skip ":" and whitespace
        let rest = rest.trim_start_matches(':').trim();
        // strip optional "Felt(...)" wrapper
        let inner = rest.strip_prefix("Felt(").and_then(|s| s.strip_suffix(')')).unwrap_or(rest);
        let inner = inner.trim().trim_end_matches(')');
        if let Ok(felt) = Felt::from_hex(inner) {
            return Ok(felt);
        }
    }
    Err(anyhow!("could not find `{label}` in saya-ops output:\n{}", output.combined))
}

fn resolve_saya_ops_bin() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("SAYA_OPS_BIN") {
        return Ok(PathBuf::from(path));
    }
    // Use `which` from the `which` crate if available; fall back to a manual
    // PATH search to keep the dep set minimal.
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("saya-ops");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(anyhow!(
        "`saya-ops` binary not found. Set SAYA_OPS_BIN env var or add it to $PATH. Build from \
         dojoengine/saya@5a3b8c9 with `cargo install --path bin/ops`."
    ))
}

fn hex(felt: &Felt) -> String {
    format!("0x{:x}", felt)
}
