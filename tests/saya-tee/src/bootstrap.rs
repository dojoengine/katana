//! L2 contract deployment via `saya-ops`.
//!
//! Shells out to the `saya-ops` binary (built from
//! `dojoengine/saya@5ff9948` — `main` post-PR-#73, with `ProgramInfo`
//! enum + `katana_tee_config_hash` plumbing) to declare and deploy:
//!
//! 1. The `mock_amd_tee_registry` contract — a permissive `IAMDTeeRegistry` mock from
//!    `cartridge-gg/piltover` (added in piltover#15), vendored into saya at
//!    `contracts/tee_registry_mock.json` and embedded in the `saya-ops` binary.
//! 2. The Piltover core contract.
//!
//! After deployment we configure the Piltover for TEE settlement directly
//! via cainome calls (`set_program_info(KatanaTee variant)` + `set_facts_registry`)
//! rather than going through `saya-ops setup-program`, which only emits the
//! `StarknetOs` variant and would cause the on-chain `validate_input` to
//! panic with cross-mode mismatch when saya-tee later submits `TeeInput`.
//!
//! `saya-ops` is resolved via `SAYA_OPS_BIN` env var or `$PATH`. Address
//! parsing scrapes the `info!`-logged "X address: Felt(0x…)" lines from
//! stdout/stderr (saya-ops uses `env_logger` which writes to stderr by
//! default; we capture both).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use katana_chain_spec::rollup::DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS;
use katana_chain_spec::tee::compute_katana_tee_config_hash;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, Call};
use starknet::macros::{selector, short_string};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_types_core::felt::Felt;

use crate::nodes::L2InProcess;

const PILTOVER_SALT: &str = "0x9117f0";
const TEE_REGISTRY_SALT: &str = "0x7ee";

/// Cairo enum variant index for `ProgramInfo::KatanaTee`. `StarknetOs` is index 0.
const KATANA_TEE_VARIANT_INDEX: Felt = Felt::ONE;

/// L3 chain id felt. Mirrors `nodes::spawn_l3`'s `ChainId::parse("KATANA")`.
/// Used to compute the `katana_tee_config_hash` that L3's `tee_generateQuote`
/// will bind into `report_data` and that on-chain Piltover asserts against.
const L3_CHAIN_ID: Felt = short_string!("KATANA");

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

    println!("Configuring Piltover for KatanaTee settlement + mock TEE registry as fact_registry");
    configure_piltover_for_tee(
        l2,
        piltover_address,
        tee_registry_address,
        account_address,
        account_private_key,
    )
    .await?;

    Ok(BootstrapResult {
        piltover_address,
        tee_registry_address,
        account_address: account_address.into(),
        account_private_key,
    })
}

/// Configures the freshly-deployed Piltover for TEE settlement, bypassing
/// `saya-ops setup-program` (which only emits the `StarknetOs` `ProgramInfo`
/// variant). Sets the `KatanaTee` variant whose `katana_tee_config_hash`
/// matches what L3's `tee_generateQuote` will compute, so the on-chain
/// `validate_input` assertion `tee_input.katana_tee_config_hash ==
/// KatanaTeeProgramInfo.katana_tee_config_hash` holds at settlement time.
///
/// Bundles `set_program_info` + `set_facts_registry` in one multicall.
async fn configure_piltover_for_tee(
    l2: &L2InProcess,
    piltover_address: Felt,
    tee_registry_address: Felt,
    account_address: Felt,
    account_private_key: Felt,
) -> Result<()> {
    let provider = l2.provider();
    let l2_chain_id =
        provider.chain_id().await.context("failed to fetch L2 chain id for account setup")?;

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(account_private_key));
    let mut account = SingleOwnerAccount::new(
        provider.clone(),
        signer,
        account_address,
        l2_chain_id,
        ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));

    let katana_tee_config_hash =
        compute_katana_tee_config_hash(L3_CHAIN_ID, DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS.into());

    // ProgramInfo::KatanaTee(KatanaTeeProgramInfo { katana_tee_config_hash })
    // serializes as [variant_index=1, katana_tee_config_hash].
    let set_program_info = Call {
        to: piltover_address,
        selector: selector!("set_program_info"),
        calldata: vec![KATANA_TEE_VARIANT_INDEX, katana_tee_config_hash],
    };
    let set_facts_registry = Call {
        to: piltover_address,
        selector: selector!("set_facts_registry"),
        calldata: vec![tee_registry_address],
    };

    let result = account
        .execute_v3(vec![set_program_info, set_facts_registry])
        .send()
        .await
        .context("set_program_info + set_facts_registry multicall failed")?;

    wait_for_tx(&provider, result.transaction_hash).await
}

async fn wait_for_tx(provider: &JsonRpcClient<HttpTransport>, tx_hash: Felt) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match provider.get_transaction_receipt(tx_hash).await {
            Ok(_) => return Ok(()),
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => return Err(anyhow!("tx {tx_hash:#x} not accepted: {e}")),
        }
    }
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
    // The bin target is `ops` (saya `bin/ops` package, renamed from `saya-ops`
    // in dojoengine/saya@5ff9948). Manual PATH search keeps the dep set minimal.
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
         dojoengine/saya@5ff9948 with `cargo install --path bin/saya-ops`."
    ))
}

fn hex(felt: &Felt) -> String {
    format!("0x{:x}", felt)
}
