//! Declare and deploy AMDTeeRegistry and KatanaTee on Starknet.
//!
//! Run `scarb build` in contracts/amd_tee_registry and contracts/katana_tee first.
//! Default contract class paths are relative to repo root.

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
use tracing::info;

// Constructor constants (match deploy_sncast.sh)
const GARAGA_CLASS_HASH: &str = "0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22";
const SP1_LOW: &str = "0xac855e58a251a65e5b78d64866896bd0";
const SP1_HIGH: &str = "0x00b7734894ae5b8056221d5d53c67f4b";
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
    let account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet> =
        SingleOwnerAccount::new(provider.clone(), signer, address, chain_id, encoding);

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

    info!(
        "Declared contracts: AMDTeeRegistry {:?}, KatanaTee {:?}",
        amd_class_hash, katana_class_hash
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

    // AMDTeeRegistry constructor calldata:
    // verifier_class_hash, sp1_program_id (low, high), max_time_diff,
    // trusted_certs (len 0), processor_models (len 2: Milan=0, Genoa=1), root_certs (len 2: milan u256, genoa u256)
    let amd_calldata = vec![
        Felt::from_hex(GARAGA_CLASS_HASH).unwrap(),
        Felt::from_hex(SP1_LOW).unwrap(),
        Felt::from_hex(SP1_HIGH).unwrap(),
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

    // KatanaTee constructor: single argument = AMDTeeRegistry address
    let katana_calldata = vec![amd_address];

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
    };

    state
        .save()
        .map_err(|e| anyhow::anyhow!("save deployment state: {}", e))?;
    info!("Deployment state saved to {}", crate::state::STATE_FILE);
    info!("  - AMDTeeRegistry: {:#064x}", amd_address);
    info!("  - KatanaTee: {:#064x}", katana_address);
    if let Some(block) = deployment_block {
        info!("  - Deployment block: {}", block);
    }
    Ok(())
}
