//! Declare and deploy AMDTeeRegistry and KatanaTee on Starknet.
//!
//! Run `scarb build` from repo root first. Before deploy, SP1 program ID is computed
//! via `cargo run -p snp-attest-cli --release -- program-id --sp1` in the SDK dir
//! (unless overridden with --sp1-program-id or --no-fetch-sp1-program-id).

use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;

use super::declare::declare_contract;
use super::deploy;
use crate::helpers::{watch_tx, POLLING_INTERVAL};
use crate::state::DeploymentState;
use anyhow::{Context, Result};
use clap::Args;
use rand::random;
use starknet_core::types::Felt;
use starknet_rust::{
    accounts::SingleOwnerAccount,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};
use tracing::{info, warn};

const GARAGA_CLASS_HASH: &str = "0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22";
/// Fallback SP1 program ID (low/high) when snp-attest-cli is not runnable.
const SP1_LOW_FALLBACK: &str = "0x1b7c8b4845b3d9ade0f084ea994f8323";
const SP1_HIGH_FALLBACK: &str = "0x00e7f4210229b46f94bd8bced85e5a1b";
const MAX_TIME_DIFF: u64 = 86400;
const MILAN_LOW: &str = "326103188097639633505521426987620764621";
const MILAN_HIGH: &str = "140650959549381881311165088169387222174";
const GENOA_LOW: &str = "122279190577630630319986709203695547121";
const GENOA_HIGH: &str = "101548849195620556729999786649524856654";

#[derive(Args, Debug, Clone)]
pub struct InitArgs {
    /// Private key for signing transactions
    #[arg(short, long, env = "PRIVATE_KEY")]
    pub private_key: String,

    /// Account address
    #[arg(short, long, env = "ACCOUNT_ADDRESS")]
    pub address: String,

    /// RPC provider URL
    #[arg(short = 'u', long, env = "PROVIDER_URL")]
    pub provider_url: String,

    /// Salt for deployment (optional; random if not set)
    #[arg(long)]
    pub salt: Option<String>,

    /// Path to AMDTeeRegistry Sierra contract class JSON (run `scarb build` from repo root first)
    #[arg(
        long,
        default_value = "target/dev/amd_tee_registry_AMDTEERegistry.contract_class.json"
    )]
    pub amd_contract_class_path: String,

    /// Path to KatanaTee Sierra contract class JSON (run `scarb build` from repo root first)
    #[arg(
        long,
        default_value = "target/dev/katana_tee_KatanaTee.contract_class.json"
    )]
    pub katana_contract_class_path: String,

    /// Path to StorageCommitment Sierra contract class JSON (run `scarb build` from repo root first)
    #[arg(
        long,
        default_value = "target/dev/storage_commitment_StorageCommitment.contract_class.json"
    )]
    pub storage_commitment_contract_class_path: String,

    /// SP1 program ID (onchain bytes32) as hex; if unset, computed via snp-attest-cli in SDK dir
    #[arg(long)]
    pub sp1_program_id: Option<String>,

    /// Do not run snp-attest-cli to fetch SP1 program ID; use fallback (requires --sp1-program-id or fallback constants)
    #[arg(long)]
    pub no_fetch_sp1_program_id: bool,

    /// Path to amd-sev-snp-attestation-sdk (for `cargo run -p snp-attest-cli -- program-id --sp1`). Default: ./crates/amd-sev-snp-attestation-sdk
    #[arg(long)]
    pub sdk_path: Option<String>,
}

pub async fn run_init(args: InitArgs) -> Result<()> {
    let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(
        Url::from_str(&args.provider_url).context("invalid provider URL")?,
    )));

    let signer: LocalWallet = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(
        Felt::from_hex(&args.private_key).context("invalid private key")?,
    ));

    let address = Felt::from_hex(&args.address).context("invalid address")?;

    let chain_id = provider
        .chain_id()
        .await
        .context("failed to fetch chain id")?;

    let encoding = starknet_rust::accounts::ExecutionEncoding::New;
    let mut account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet> =
        SingleOwnerAccount::new(provider.clone(), signer, address, chain_id, encoding);
    // Use pre-confirmed block for nonce to avoid nonce mismatch after waiting for tx confirmation
    account.set_block_id(starknet_core::types::BlockId::Tag(
        starknet_core::types::BlockTag::PreConfirmed,
    ));

    // Declare AMDTeeRegistry
    let (maybe_tx, amd_class_hash) = declare_contract(&account, &args.amd_contract_class_path)
        .await
        .map_err(|e| anyhow::anyhow!("declare AMDTeeRegistry: {}", e))?;

    if let Some(tx) = maybe_tx {
        info!("Waiting for AMDTeeRegistry declaration to be confirmed...");
        let _ = watch_tx(&provider, tx.transaction_hash, POLLING_INTERVAL).await;
    }

    // Declare KatanaTee
    let (maybe_tx, katana_class_hash) =
        declare_contract(&account, &args.katana_contract_class_path)
            .await
            .map_err(|e| anyhow::anyhow!("declare KatanaTee: {}", e))?;

    if let Some(ref tx) = maybe_tx {
        info!("Waiting for KatanaTee declaration to be confirmed...");
        let _ = watch_tx(&provider, tx.transaction_hash, POLLING_INTERVAL).await;
    }

    // Declare StorageCommitment
    let (maybe_tx, storage_commitment_class_hash) =
        declare_contract(&account, &args.storage_commitment_contract_class_path)
            .await
            .map_err(|e| anyhow::anyhow!("declare StorageCommitment: {}", e))?;

    if let Some(ref tx) = maybe_tx {
        info!("Waiting for StorageCommitment declaration to be confirmed...");
        let _ = watch_tx(&provider, tx.transaction_hash, POLLING_INTERVAL).await;
    }

    info!(
        "Declared contracts: AMDTeeRegistry {:?}, KatanaTee {:?}, StorageCommitment {:?}",
        amd_class_hash, katana_class_hash, storage_commitment_class_hash
    );

    let salt = if let Some(ref salt_hex) = args.salt {
        Felt::from_hex(salt_hex).context("invalid salt hex format")?
    } else {
        let random_bytes: [u8; 32] = random();
        let hex_string = format!(
            "0x{}",
            random_bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        );
        Felt::from_hex_unchecked(&hex_string)
    };
    info!("Using salt for deployment: {:#064x}", salt);

    let (sp1_low, sp1_high) = resolve_sp1_program_id(&args)?;
    info!(
        "SP1 program ID: low {:#064x}, high {:#064x}",
        sp1_low, sp1_high
    );

    // AMDTeeRegistry constructor calldata:
    // verifier_class_hash, sp1_program_id (low, high), max_time_diff,
    // trusted_certs (len 0), processor_models (len 2: Milan=0, Genoa=1), root_certs (len 2: milan u256, genoa u256)
    let amd_calldata = vec![
        Felt::from_hex(GARAGA_CLASS_HASH).unwrap(),
        sp1_low,
        sp1_high,
        Felt::from(MAX_TIME_DIFF),
        Felt::ZERO,       // trusted_certs len
        Felt::from(2u64), // processor_models len
        Felt::ZERO,       // Milan
        Felt::ONE,        // Genoa
        Felt::from(2u64), // root_certs len
        Felt::from_dec_str(MILAN_LOW).unwrap(),
        Felt::from_dec_str(MILAN_HIGH).unwrap(),
        Felt::from_dec_str(GENOA_LOW).unwrap(),
        Felt::from_dec_str(GENOA_HIGH).unwrap(),
    ];

    let (maybe_tx, amd_address) =
        deploy::deploy(&account, amd_class_hash, amd_calldata, Some(salt), false)
            .await
            .map_err(|e| anyhow::anyhow!("deploy AMDTeeRegistry: {}", e))?;

    info!(
        "Deployed AMDTeeRegistry: {:?}, tx_hash: {:?}",
        amd_address, maybe_tx
    );

    if let Some(ref tx_result) = maybe_tx {
        info!("Waiting for AMDTeeRegistry deployment to be confirmed...");
        let _ = watch_tx(&provider, tx_result.transaction_hash, POLLING_INTERVAL).await;
    }

    // Deploy StorageCommitment (no constructor arguments)
    let storage_commitment_calldata = vec![];

    let (maybe_tx, storage_commitment_address) = deploy::deploy(
        &account,
        storage_commitment_class_hash,
        storage_commitment_calldata,
        Some(salt),
        false,
    )
    .await
    .map_err(|e| anyhow::anyhow!("deploy StorageCommitment: {}", e))?;

    info!(
        "Deployed StorageCommitment: {:?}, tx_hash: {:?}",
        storage_commitment_address, maybe_tx
    );

    if let Some(ref tx_result) = maybe_tx {
        info!("Waiting for StorageCommitment deployment to be confirmed...");
        let _ = watch_tx(&provider, tx_result.transaction_hash, POLLING_INTERVAL).await;
    }

    // KatanaTee constructor: registry_address, storage_commitment_registry
    let katana_calldata = vec![amd_address, storage_commitment_address];

    let (maybe_tx, katana_address) = deploy::deploy(
        &account,
        katana_class_hash,
        katana_calldata,
        Some(salt),
        false,
    )
    .await
    .map_err(|e| anyhow::anyhow!("deploy KatanaTee: {}", e))?;

    info!(
        "Deployed KatanaTee: {:?}, tx_hash: {:?}",
        katana_address, maybe_tx
    );

    let deployment_block = if let Some(tx_result) = maybe_tx {
        info!("Waiting for KatanaTee deployment to be confirmed...");
        let receipt = watch_tx(&provider, tx_result.transaction_hash, POLLING_INTERVAL)
            .await
            .map_err(|e| anyhow::anyhow!("wait for KatanaTee deployment: {}", e))?;
        let block_number = receipt.block.block_number();
        info!("KatanaTee deployed at block: {}", block_number);
        Some(block_number)
    } else {
        info!("KatanaTee was already deployed, deployment block unknown");
        None
    };

    let state = DeploymentState {
        deployment_block,
        amd_tee_registry_address: Some(format!("{:#064x}", amd_address)),
        katana_tee_address: Some(format!("{:#064x}", katana_address)),
        storage_commitment_address: Some(format!("{:#064x}", storage_commitment_address)),
    };

    state
        .save()
        .map_err(|e| anyhow::anyhow!("save deployment state: {}", e))?;
    info!("Deployment state saved to {}", crate::state::STATE_FILE);
    info!("  - AMDTeeRegistry: {:#064x}", amd_address);
    info!(
        "  - StorageCommitment: {:#064x}",
        storage_commitment_address
    );
    info!("  - KatanaTee: {:#064x}", katana_address);
    if let Some(block) = deployment_block {
        info!("  - Deployment block: {}", block);
    }
    Ok(())
}

/// Resolve SP1 program ID: from --sp1-program-id, or by running snp-attest-cli, or fallback constants.
/// Returns (low, high) as u256 for constructor calldata (low = last 16 bytes, high = first 16 bytes).
fn resolve_sp1_program_id(args: &InitArgs) -> Result<(Felt, Felt)> {
    if let Some(ref hex_id) = args.sp1_program_id {
        info!("Using SP1 program ID from --sp1-program-id argument");
        return parse_program_id_hex(hex_id).context("invalid --sp1-program-id hex");
    }
    if !args.no_fetch_sp1_program_id {
        match fetch_sp1_program_id_from_cli(args.sdk_path.as_deref()) {
            Ok((low, high)) => {
                info!("Using SP1 program ID fetched from snp-attest-cli");
                return Ok((low, high));
            }
            Err(e) => {
                warn!("Failed to fetch SP1 program ID from snp-attest-cli: {:#}", e);
            }
        }
    }
    warn!(
        "WARNING: Using HARDCODED FALLBACK SP1 program ID! This may not match the current SP1 circuit. \
         Use --sp1-program-id to specify the correct value, or run from repo root so snp-attest-cli can be found."
    );
    warn!(
        "Fallback SP1 program ID: high={} low={}",
        SP1_HIGH_FALLBACK, SP1_LOW_FALLBACK
    );
    Ok((
        Felt::from_hex(SP1_LOW_FALLBACK).context("fallback SP1 low")?,
        Felt::from_hex(SP1_HIGH_FALLBACK).context("fallback SP1 high")?,
    ))
}

/// Parse "0x" + 64 hex chars into (low, high) felt. Low = last 16 bytes, high = first 16 bytes.
fn parse_program_id_hex(hex_id: &str) -> Result<(Felt, Felt)> {
    let s = hex_id.strip_prefix("0x").unwrap_or(hex_id);
    let s = s.trim();
    anyhow::ensure!(
        s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()),
        "SP1 program ID must be 32 bytes (64 hex chars)"
    );
    let low_hex = format!("0x{}", &s[32..]);
    let high_hex = format!("0x{}", &s[..32]);
    Ok((
        Felt::from_hex(&low_hex).context("SP1 low")?,
        Felt::from_hex(&high_hex).context("SP1 high")?,
    ))
}

/// Run `cargo run -p snp-attest-cli --release -- program-id --sp1` in SDK dir and parse onchain representation.
fn fetch_sp1_program_id_from_cli(sdk_path_opt: Option<&str>) -> Result<(Felt, Felt)> {
    let sdk_path = match sdk_path_opt {
        Some(p) => Path::new(p).to_path_buf(),
        None => {
            let cwd = std::env::current_dir().context("current_dir")?;
            let default = cwd.join("crates").join("amd-sev-snp-attestation-sdk");
            if default.exists() {
                default
            } else {
                anyhow::bail!(
                    "SDK path not found: {}. Set --sdk-path or run from repo root",
                    default.display()
                );
            }
        }
    };
    let output = Command::new("cargo")
        .args([
            "run",
            "-p",
            "snp-attest-cli",
            "--release",
            "--",
            "program-id",
            "--sp1",
        ])
        .current_dir(&sdk_path)
        .output()
        .context("run snp-attest-cli")?;
    anyhow::ensure!(
        output.status.success(),
        "snp-attest-cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let prefix = "ProgramID (Onchain Representation): ";
    let line = stdout
        .lines()
        .find(|l| l.starts_with(prefix))
        .context("snp-attest-cli output missing onchain program ID line")?;
    let hex_id = line.strip_prefix(prefix).context("prefix")?.trim();
    parse_program_id_hex(hex_id)
}
