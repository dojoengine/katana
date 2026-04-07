//! End-to-end test that runs Saya's `saya-tee` binary against an in-process
//! Katana, exercising the persistent-TEE settlement path with all proving
//! mocked out.
//!
//! ## Pipeline
//!
//! 1. Spawn an L2 dev Katana via [`katana_utils::TestNode`].
//! 2. Shell out to `saya-ops` to declare and deploy:
//!    - `mock_amd_tee_registry` (the on-chain mock from cartridge-gg/piltover#15), and
//!    - the Piltover core contract pointed at the mock registry.
//! 3. Spawn an L3 rollup Katana via `TestNode` with `SettlementLayer::Starknet { … }` pointing at
//!    L2's Piltover, and `TeeConfig { provider_type: Mock, .. }` so its `tee_generateQuote` RPC
//!    serves a stub attestation.
//! 4. Spawn `saya-tee tee start --mock-prove` as a child process pointed at both Katanas. The flag
//!    (added in dojoengine/saya#60) makes saya-tee skip AMD KDS, cert chain validation, and SP1
//!    proving entirely.
//! 5. Drive a few L3 blocks by submitting no-op transfers.
//! 6. Poll Piltover's `get_state()` until `block_number != Felt::MAX`, proving that saya-tee
//!    successfully settled L3 state to L2.
//!
//! ## Required binaries
//!
//! - `saya-ops`: discovered via `SAYA_OPS_BIN` env var or `$PATH`. Built from dojoengine/saya
//!   `feat/mock-prove`.
//! - `saya-tee`: discovered via `SAYA_TEE_BIN` env var or `$PATH`. Built from dojoengine/saya
//!   `feat/mock-prove`.

use std::time::Duration;

use anyhow::Result;
use tracing::{info, warn};

mod assertions;
mod bootstrap;
mod nodes;
mod saya;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    init_logging();

    info!("=== saya-tee e2e test starting ===");

    // 1. Spawn L2 dev Katana.
    let l2 = nodes::spawn_l2().await;
    info!(l2_url = %l2.url(), "L2 Katana ready");

    // 2. Bootstrap mock TEE registry + Piltover on L2 via saya-ops.
    let bootstrap = bootstrap::bootstrap_l2(&l2).await?;
    info!(
        piltover = %hex_felt(&bootstrap.piltover_address),
        tee_registry = %hex_felt(&bootstrap.tee_registry_address),
        "L2 contracts deployed"
    );

    // 3. Spawn L3 rollup Katana with TEE config + settlement pointed at L2.
    let l3 = nodes::spawn_l3(&l2, bootstrap.piltover_address).await;
    info!(l3_url = %l3.url(), "L3 Katana ready");

    // 4. Spawn saya-tee --mock-prove as a sidecar (RAII guard kills on drop).
    let _saya = saya::spawn_saya_tee(&saya::SayaTeeConfig {
        rollup_rpc: l3.url(),
        settlement_rpc: l2.url(),
        piltover_address: bootstrap.piltover_address,
        tee_registry_address: bootstrap.tee_registry_address,
        settlement_account_address: bootstrap.account_address,
        settlement_account_private_key: bootstrap.account_private_key,
    })?;
    info!("saya-tee sidecar spawned");

    // 5. Drive L3 to advance block height.
    nodes::drive_l3_blocks(&l3, 3).await?;
    info!("L3 advanced to block height >= 3");

    // 6. Wait for Piltover state to advance past the genesis sentinel.
    assertions::wait_for_settlement(&l2, bootstrap.piltover_address, Duration::from_secs(180))
        .await?;

    info!("=== saya-tee e2e test PASSED ===");
    Ok(())
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,saya_tee_e2e_test=debug,katana_node=warn,katana_core=warn")
    });
    if let Err(e) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        warn!("failed to init tracing subscriber: {e}");
    }
}

fn hex_felt(felt: &starknet_types_core::felt::Felt) -> String {
    format!("0x{:x}", felt)
}
