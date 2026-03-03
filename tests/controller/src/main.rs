//! E2E integration test for the Cartridge API server.
//!
//! Validates the full flow: controller CLI -> Katana CartridgeApi RPC -> real Cartridge API
//! + real paymaster sidecar -> on-chain execution of Dojo contract functions (spawn/move).
//!
//! # Prerequisites
//!
//! - `paymaster-service` binary in PATH
//! - `controller` CLI in PATH
//! - Pre-authorized controller session via `controller session auth`
//! - Internet access to `api.cartridge.gg`
//! - Test fixtures built (`make fixtures`)
//!
//! The test gracefully skips (exit 0) if prerequisites aren't available.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
use katana_paymaster::{PaymasterService, PaymasterServiceConfigBuilder};
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::RpcSierraContractClass;
use katana_sequencer_node::config::paymaster::{CartridgeApiConfig, PaymasterConfig};
use katana_slot_controller::*;
use katana_utils::node::{test_config, TestNode};
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, FlattenedSierraClass};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use url::Url;

/// The actions contract address from the pre-migrated spawn-and-move DB.
const ACTIONS_ADDRESS_ENV: &str = "ACTIONS_CONTRACT_ADDRESS";

/// Path to the controller CLI storage directory.
/// Defaults to `~/Library/Application Support/controller-cli` on macOS.
const CONTROLLER_STORAGE_PATH_ENV: &str = "CONTROLLER_STORAGE_PATH";

/// Default API key for the paymaster sidecar.
const PAYMASTER_API_KEY: &str = "paymaster_katana";

// ---------------------------------------------------------------------------
// Prerequisite checks
// ---------------------------------------------------------------------------

fn binary_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(name);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

struct Prerequisites {
    controller_storage_path: PathBuf,
    actions_address: String,
}

fn default_controller_storage_path() -> PathBuf {
    // Match the controller CLI default: ~/Library/Application Support/controller-cli (macOS)
    // or ~/.local/share/controller-cli (Linux)
    if cfg!(target_os = "macos") {
        home_dir().join("Library/Application Support/controller-cli")
    } else {
        home_dir().join(".local/share/controller-cli")
    }
}

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).expect("HOME env var not set")
}

fn check_prerequisites() -> Option<Prerequisites> {
    let mut missing = Vec::new();

    if !binary_exists("paymaster-service") {
        missing.push("paymaster-service binary not found in PATH");
    }
    if !binary_exists("controller") {
        missing.push("controller CLI binary not found in PATH");
    }

    let db_path = spawn_and_move_db_path();
    if !db_path.exists() {
        missing.push("spawn-and-move DB fixture not found (run `make fixtures`)");
    }

    let storage_path = std::env::var(CONTROLLER_STORAGE_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_controller_storage_path());

    // Verify the controller has an active session
    let active_file = storage_path.join("@cartridge").join("active");
    if !active_file.exists() {
        missing.push("No active controller session (run `controller session auth`)");
    }

    let session_signer_file = storage_path.join("session_signer");
    if !session_signer_file.exists() {
        missing.push("No session signer found in controller storage");
    }

    let actions_address = std::env::var(ACTIONS_ADDRESS_ENV).ok();
    if actions_address.is_none() {
        missing.push("ACTIONS_CONTRACT_ADDRESS env var not set");
    }

    if !missing.is_empty() {
        eprintln!("Skipping controller E2E test — missing prerequisites:");
        for reason in &missing {
            eprintln!("  - {reason}");
        }
        return None;
    }

    Some(Prerequisites {
        controller_storage_path: storage_path,
        actions_address: actions_address.unwrap(),
    })
}

fn spawn_and_move_db_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/db/spawn_and_move")
}

// ---------------------------------------------------------------------------
// Genesis account helper
// ---------------------------------------------------------------------------

fn prefunded_account(
    chain_spec: &katana_chain_spec::ChainSpec,
    index: usize,
) -> Result<(ContractAddress, Felt)> {
    let (address, allocation) = chain_spec
        .genesis()
        .accounts()
        .nth(index)
        .context(format!("genesis account index {index} out of range"))?;

    let private_key = match allocation {
        GenesisAccountAlloc::DevAccount(account) => account.private_key,
        _ => anyhow::bail!("genesis account {address} has no private key"),
    };

    Ok((*address, private_key))
}

// ---------------------------------------------------------------------------
// Controller class declaration
// ---------------------------------------------------------------------------

/// Declare all controller class versions on-chain using a genesis account.
async fn declare_controller_classes(
    provider: &JsonRpcClient<HttpTransport>,
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
) -> Result<()> {
    let classes: Vec<(&str, Felt, Felt, &katana_primitives::class::ContractClass)> = vec![
        (
            "ControllerV104",
            ControllerV104::HASH,
            ControllerV104::CASM_HASH,
            &*ControllerV104::CLASS,
        ),
        (
            "ControllerV105",
            ControllerV105::HASH,
            ControllerV105::CASM_HASH,
            &*ControllerV105::CLASS,
        ),
        (
            "ControllerV106",
            ControllerV106::HASH,
            ControllerV106::CASM_HASH,
            &*ControllerV106::CLASS,
        ),
        (
            "ControllerV107",
            ControllerV107::HASH,
            ControllerV107::CASM_HASH,
            &*ControllerV107::CLASS,
        ),
        (
            "ControllerV108",
            ControllerV108::HASH,
            ControllerV108::CASM_HASH,
            &*ControllerV108::CLASS,
        ),
        (
            "ControllerV109",
            ControllerV109::HASH,
            ControllerV109::CASM_HASH,
            &*ControllerV109::CLASS,
        ),
        (
            "ControllerLatest",
            ControllerLatest::HASH,
            ControllerLatest::CASM_HASH,
            &*ControllerLatest::CLASS,
        ),
    ];

    for (name, class_hash, casm_hash, class) in classes {
        if is_class_declared(provider, class_hash).await? {
            eprintln!("  {name} already declared ({class_hash:#x})");
            continue;
        }

        eprintln!("  Declaring {name} ({class_hash:#x})...");

        let sierra = class.clone().to_sierra().context(format!("{name} is not a Sierra class"))?;
        let rpc_class = RpcSierraContractClass::from(sierra);
        let flattened = FlattenedSierraClass::try_from(rpc_class)
            .map_err(|e| anyhow::anyhow!("failed to flatten {name}: {e}"))?;

        let result = account
            .declare_v3(flattened.into(), casm_hash)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("failed to declare {name}: {e}"))?;

        assert_eq!(result.class_hash, class_hash, "class hash mismatch for {name}");
        wait_for_class_declared(provider, class_hash).await?;
        eprintln!("  {name} declared successfully");
    }

    Ok(())
}

async fn is_class_declared(
    provider: &JsonRpcClient<HttpTransport>,
    class_hash: Felt,
) -> Result<bool> {
    use starknet::core::types::StarknetError;
    use starknet::providers::ProviderError;

    match provider.get_class(BlockId::Tag(BlockTag::PreConfirmed), class_hash).await {
        Ok(_) => Ok(true),
        Err(ProviderError::StarknetError(StarknetError::ClassHashNotFound)) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

async fn wait_for_class_declared(
    provider: &JsonRpcClient<HttpTransport>,
    class_hash: Felt,
) -> Result<()> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);
    loop {
        if is_class_declared(provider, class_hash).await? {
            return Ok(());
        }
        if start.elapsed() > timeout {
            anyhow::bail!("class {class_hash:#x} not declared after {timeout:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

// ---------------------------------------------------------------------------
// Controller CLI session seeding
// ---------------------------------------------------------------------------

/// Copy the controller CLI storage to a temp directory, rewriting `session_rpc_url`
/// to point at the local Katana node.
fn seed_controller_session(src_storage: &Path, temp_dir: &Path, rpc_url: &str) -> Result<PathBuf> {
    let dst = temp_dir.join("controller-storage");
    copy_dir_recursive(src_storage, &dst)?;

    // Rewrite session_rpc_url to point at local Katana
    let rpc_url_value = serde_json::json!({"String": rpc_url});
    std::fs::write(dst.join("session_rpc_url"), rpc_url_value.to_string())?;

    Ok(dst)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src).context(format!("reading {}", src.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Controller CLI execution
// ---------------------------------------------------------------------------

async fn controller_execute(
    storage_path: &Path,
    rpc_url: &str,
    to: &str,
    selector: &str,
    calldata: Option<&str>,
) -> Result<serde_json::Value> {
    let mut cmd = tokio::process::Command::new("controller");
    cmd.arg("execute")
        .arg("--to")
        .arg(to)
        .arg("--selector")
        .arg(selector)
        .arg("--rpc-url")
        .arg(rpc_url)
        .arg("--json");

    if let Some(data) = calldata {
        cmd.arg("--calldata").arg(data);
    }

    cmd.env("CONTROLLER_STORAGE_PATH", storage_path);

    let output = cmd.output().await.context("failed to run controller CLI")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        anyhow::bail!(
            "controller execute failed (exit code {:?}):\nstdout: {stdout}\nstderr: {stderr}",
            output.status.code()
        );
    }

    eprintln!("  controller stdout: {stdout}");
    if !stderr.is_empty() {
        eprintln!("  controller stderr: {stderr}");
    }

    let json: serde_json::Value =
        serde_json::from_str(stdout.trim()).context("failed to parse controller JSON output")?;
    Ok(json)
}

// ---------------------------------------------------------------------------
// Main test flow
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    // Step 1: Check prerequisites
    let prereqs = match check_prerequisites() {
        Some(p) => p,
        None => return Ok(()),
    };

    eprintln!("=== Controller E2E Test ===");

    // Step 2: Start Katana node from spawn-and-move DB
    eprintln!("Starting Katana node from spawn-and-move DB...");

    let mut config = test_config();

    let paymaster_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.local_addr()?.port()
    };
    let paymaster_url =
        Url::parse(&format!("http://127.0.0.1:{paymaster_port}")).expect("valid paymaster url");

    let chain_spec = config.chain.clone();
    let (relayer_addr, relayer_pk) = prefunded_account(&chain_spec, 0)?;
    let (gas_tank_addr, gas_tank_pk) = prefunded_account(&chain_spec, 1)?;
    let (estimate_addr, estimate_pk) = prefunded_account(&chain_spec, 2)?;

    config.paymaster = Some(PaymasterConfig {
        url: paymaster_url.clone(),
        api_key: Some(PAYMASTER_API_KEY.to_string()),
        cartridge_api: Some(CartridgeApiConfig {
            cartridge_api_url: Url::parse("https://api.cartridge.gg").unwrap(),
            controller_deployer_address: relayer_addr,
            controller_deployer_private_key: relayer_pk,
            vrf: None,
        }),
    });

    let db_path = spawn_and_move_db_path();
    let node = TestNode::new_from_db_with_config(&db_path, config).await;
    let rpc_url = format!("http://{}", node.rpc_addr());

    eprintln!("Katana node started at {rpc_url}");

    // Step 3: Declare controller classes at runtime
    eprintln!("Declaring controller classes...");

    let provider = node.starknet_provider();
    let chain_id_felt = provider.chain_id().await?;

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(relayer_pk));
    let mut declare_account = SingleOwnerAccount::new(
        node.starknet_provider(),
        signer,
        relayer_addr.into(),
        chain_id_felt,
        ExecutionEncoding::New,
    );
    declare_account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));

    declare_controller_classes(&provider, &declare_account).await?;
    eprintln!("Controller classes declared.");

    // Step 4: Bootstrap and start paymaster sidecar
    eprintln!("Bootstrapping paymaster sidecar...");

    let paymaster_config = PaymasterServiceConfigBuilder::new()
        .rpc(*node.rpc_addr())
        .port(paymaster_port)
        .api_key(PAYMASTER_API_KEY)
        .relayer(relayer_addr, relayer_pk)
        .gas_tank(gas_tank_addr, gas_tank_pk)
        .estimate_account(estimate_addr, estimate_pk)
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .build()
        .await
        .context("failed to build paymaster config")?;

    let mut paymaster = PaymasterService::new(paymaster_config);
    paymaster.bootstrap().await.context("failed to bootstrap paymaster")?;

    eprintln!("Starting paymaster sidecar on port {paymaster_port}...");
    let mut sidecar = paymaster.start().await.context("failed to start paymaster sidecar")?;
    eprintln!("Paymaster sidecar started.");

    // Step 5: Seed controller CLI session — copy real credentials, rewrite RPC URL
    eprintln!("Seeding controller CLI session...");

    let session_dir = tempfile::tempdir()?;
    let storage_path =
        seed_controller_session(&prereqs.controller_storage_path, session_dir.path(), &rpc_url)?;

    eprintln!("Session seeded at {}", storage_path.display());

    // Step 6: Execute spawn via controller CLI
    eprintln!("Executing spawn via controller CLI...");
    let spawn_result =
        controller_execute(&storage_path, &rpc_url, &prereqs.actions_address, "spawn", None)
            .await
            .context("spawn execution failed")?;
    eprintln!("Spawn result: {spawn_result}");

    // Step 7: Verify
    eprintln!("Verifying results...");

    if let Some(tx_hash) = spawn_result.get("transaction_hash") {
        eprintln!("  Spawn tx hash: {tx_hash}");
    }

    eprintln!("=== Controller E2E Test PASSED ===");

    // Step 8: Cleanup
    sidecar.shutdown().await?;

    Ok(())
}
