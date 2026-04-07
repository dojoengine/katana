//! Katana node spawning for the saya-tee e2e test.
//!
//! Spawns two Katanas:
//! - **L2** — vanilla dev chain spawned as a `katana --dev` **subprocess**. Acts as the settlement
//!   chain that hosts Piltover and the mock TEE registry.
//!
//!   The L2 must run in a separate process because Katana's executor uses a process-global
//!   blockifier `ContractClassManager` (see `crates/executor/src/blockifier/cache.rs`'s
//!   `COMPILED_CLASS_CACHE: OnceLock<ClassCache>`). When saya-ops invokes UDC on L2 to deploy
//!   Piltover and the mock TEE registry, blockifier lazy-loads UDC's compiled class into that
//!   global cache. If the L3 then runs in the same process, its `GenesisTransactionsBuilder`'s
//!   `Declare` txs for UDC/Account/ERC20 fail with "already declared" because the global cache
//!   already has them. Subprocessing the L2 keeps each blockifier instance per-process.
//!
//! - **L3** — rollup chain whose `SettlementLayer::Starknet` points at L2's Piltover address.
//!   Spawned **in-process** via [`katana_utils::TestNode`] so we can configure
//!   `Config.tee = TeeConfig { provider_type: Mock, .. }` (the katana CLI doesn't currently
//!   expose `--tee.provider mock`, only `sev-snp`). Its `tee_generateQuote` RPC serves a stub
//!   attestation that `saya-tee --mock-prove` consumes.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use katana_chain_spec::rollup::DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS;
use katana_chain_spec::{rollup, ChainSpec, FeeContracts, SettlementLayer};
use katana_genesis::allocation::DevAllocationsGenerator;
use katana_genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::U256;
use katana_sequencer_node::config::tee::TeeConfig;
use katana_tee::TeeProviderType;
use katana_utils::TestNode;
use starknet::accounts::SingleOwnerAccount;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet::signers::LocalWallet;
use starknet_types_core::felt::Felt;
use tracing::{debug, info, warn};
use url::Url;

/// L2 settlement Katana, spawned as an external `katana --dev` subprocess
/// (see module docs for why).
pub struct L2Subprocess {
    child: Child,
    url: Url,
}

impl Drop for L2Subprocess {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(status)) => debug!(?status, "katana L2 already exited before drop"),
            Ok(None) => {
                if let Err(e) = self.child.kill() {
                    warn!(error = %e, "failed to kill katana L2 child");
                }
                let _ = self.child.wait();
            }
            Err(e) => warn!(error = %e, "failed to query katana L2 child status"),
        }
    }
}

impl L2Subprocess {
    pub fn url(&self) -> Url {
        self.url.clone()
    }

    pub fn provider(&self) -> JsonRpcClient<HttpTransport> {
        JsonRpcClient::new(HttpTransport::new(self.url.clone()))
    }

    /// The first prefunded katana dev account. The dev chain spec uses a
    /// deterministic seed, so this address and key are stable across runs.
    /// Source: same `DevAllocationsGenerator` that `dev::ChainSpec::default()`
    /// uses internally.
    pub fn prefunded_account_keys(&self) -> (Felt, Felt) {
        // First prefunded account in `katana --dev`, deterministic. Identical
        // to what Saya's upstream e2e harness (`compose.l2.yml`) uses for
        // `katana0`. Hardcoded here so we don't need to query the L2 or
        // depend on the internal `dev::ChainSpec` allocation seed.
        let address = Felt::from_hex(
            "0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec",
        )
        .expect("hardcoded address parses");
        let private_key = Felt::from_hex(
            "0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912",
        )
        .expect("hardcoded key parses");
        (address, private_key)
    }
}

/// L3 rollup Katana, spawned in-process via [`TestNode`].
pub struct L3InProcess {
    inner: TestNode,
}

impl L3InProcess {
    pub fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.inner.rpc_addr()))
            .expect("rpc_addr produces valid URL")
    }

    pub fn provider(&self) -> JsonRpcClient<HttpTransport> {
        self.inner.starknet_provider()
    }

    pub fn account(&self) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
        self.inner.account()
    }
}

/// Spawns the L2 settlement Katana as an external `katana --dev` subprocess.
///
/// Resolves the binary via `KATANA_BIN` env var or falls back to
/// `target/debug/katana` relative to `CARGO_MANIFEST_DIR/../..` (the workspace
/// root). The caller is responsible for ensuring the binary exists — run
/// `cargo build -p katana --bin katana` first.
pub async fn spawn_l2() -> Result<L2Subprocess> {
    let bin = resolve_katana_bin()?;
    let port = pick_free_port()?;

    info!(?bin, port, "spawning katana L2 subprocess");
    let child = Command::new(&bin)
        .args([
            "--dev",
            "--dev.no-fee",
            "--http.addr",
            "127.0.0.1",
            "--http.port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn katana L2 at {bin:?}"))?;

    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).expect("valid url");

    // Wait for the L2 RPC to start accepting connections.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() >= deadline {
            return Err(anyhow!("katana L2 did not become reachable on {url} within 30s"));
        }
        if std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(200),
        )
        .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    Ok(L2Subprocess { child, url })
}

fn resolve_katana_bin() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("KATANA_BIN") {
        return Ok(PathBuf::from(path));
    }

    // Default to target/debug/katana relative to workspace root.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = PathBuf::from(manifest_dir).join("../..");
    let candidate = workspace_root.join("target/debug/katana");
    if candidate.is_file() {
        return Ok(candidate);
    }

    Err(anyhow!(
        "`katana` binary not found. Set KATANA_BIN env var or run \
         `cargo build -p katana --bin katana` first. Tried: {}",
        candidate.display()
    ))
}

fn pick_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to bind to ephemeral port")?;
    let port = listener.local_addr().context("failed to read local addr")?.port();
    drop(listener);
    Ok(port)
}

/// Spawns the L3 rollup Katana with TEE config and settlement pointed at L2.
///
/// Constructs a [`rollup::ChainSpec`] with `SettlementLayer::Starknet`
/// referencing the L2 Piltover address, and a [`Config`] with the mock TEE
/// provider enabled so `tee_generateQuote` works without real SEV-SNP
/// hardware.
pub async fn spawn_l3(l2: &L2Subprocess, piltover_address: Felt) -> L3InProcess {
    let l2_url = l2.url();
    let l2_chain_id = ChainId::SEPOLIA; // Matches katana_utils::test_config()'s default.

    // Use the default appchain fee token so it matches what saya assumes
    // for the StarknetOsConfig hash computation. (saya-tee `--mock-prove`
    // doesn't actually rely on this, but keeping it canonical avoids
    // accidental mismatches if the flag is removed in the future.)
    let fee_contracts = FeeContracts {
        eth: DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS,
        strk: DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS,
    };

    // Generate prefunded accounts for the L3 so the test can drive
    // transactions through it.
    let accounts = DevAllocationsGenerator::new(10)
        .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
        .generate();
    // IMPORTANT: rollup chain specs run their genesis through
    // `GenesisTransactionsBuilder`, which emits explicit declare txs for the
    // ERC20, UDC, and account classes. `Genesis::default()` pre-declares
    // those same three classes directly into chain state, which causes the
    // builder's declare txs to fail with "already declared", cascading into
    // nonce mismatches that skip the `transfer_balance` calls and leave
    // prefunded accounts with zero balance. So we start from an empty-classes
    // genesis here.
    let mut genesis = Genesis { classes: Default::default(), ..Genesis::default() };
    genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

    let settlement = SettlementLayer::Starknet {
        block: 0,
        id: l2_chain_id,
        rpc_url: l2_url,
        core_contract: piltover_address.into(),
    };

    let l3_chain = rollup::ChainSpec {
        id: ChainId::parse("KATANA").expect("KATANA is a valid chain id"),
        genesis,
        fee_contracts,
        settlement,
    };

    let mut config = katana_utils::node::test_config();
    config.chain = Arc::new(ChainSpec::Rollup(l3_chain));
    config.tee = Some(TeeConfig { provider_type: TeeProviderType::Mock, fork_block_number: None });
    // Note: rollup chain specs (provable mode) never produce empty blocks
    // even with `block_time` set, per the upstream Saya README. The L3 only
    // advances when transactions are submitted; we drive that explicitly
    // via [`drive_l3_blocks`] below.

    L3InProcess { inner: TestNode::new_with_config(config).await }
}

/// Drives the L3 forward by submitting `n` no-op self-transfers via the
/// prefunded test account. Each transfer triggers a new block (provable-mode
/// rollups never produce empty blocks, so transactions are the only way to
/// advance height).
pub async fn drive_l3_blocks(l3: &L3InProcess, n: u64) -> Result<()> {
    use starknet::accounts::{Account, ConnectedAccount};
    use starknet::core::types::Call;
    use starknet::macros::selector;

    let account = l3.account();
    let address = account.address();
    let strk: Felt = DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS.into();

    for i in 0..n {
        let call = Call {
            to: strk,
            selector: selector!("transfer"),
            calldata: vec![address, Felt::from(1u64), Felt::ZERO],
        };

        let result = account
            .execute_v3(vec![call])
            .send()
            .await
            .with_context(|| format!("driver tx {i} failed to send"))?;

        wait_for_tx(&l3.provider(), result.transaction_hash).await?;
    }

    Ok(())
}

async fn wait_for_tx(provider: &JsonRpcClient<HttpTransport>, tx_hash: Felt) -> Result<()> {
    use starknet::providers::Provider;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match provider.get_transaction_receipt(tx_hash).await {
            Ok(_) => return Ok(()),
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => return Err(anyhow::anyhow!("tx {tx_hash:#x} not accepted: {e}")),
        }
    }
}
