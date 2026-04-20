//! End-to-end test for Saya's persistent-TEE settlement path, with all cryptographic
//! components swapped for permissive stubs.
//!
//! ## What's real
//!
//! - L2 dev Katana and L3 rollup Katana, both in-process.
//! - Piltover core contract on L2 (real Cairo, real state-transition math).
//! - `saya-ops` subprocess: declares and deploys Piltover + the TEE registry on L2.
//! - `saya-tee` subprocess: polls L3, builds settlement transactions, submits `update_state` to L2.
//! - The state-diff → Poseidon commitment → `report_data` → `validate_input` round-trip.
//!
//! ## What's mocked
//!
//! | Component | Real | Mock |
//! |-----------|------|------|
//! | `tee_generateQuote` on L3 | AMD SEV-SNP hardware-signed quote | `katana_tee::MockProvider`: stub quote. `report_data` is a real Poseidon commitment over the state diff; only the hardware signature is absent. |
//! | SP1 proving in saya-tee | Real SP1 proof over the state diff | `--mock-prove` synthesizes a stub `OnchainProof`. SP1 prover network is never contacted. |
//! | AMD KDS + cert-chain verification | saya-tee walks AMD root → VCEK | Skipped by `--mock-prove`. |
//! | On-chain fact registry | Runs SP1 verifier in Cairo | `mock_amd_tee_registry` (piltover#15): returns the SP1 journal verbatim. |
//!
//! ## What this proves
//!
//! - End-to-end **plumbing** between L3, saya-tee, Piltover, and L2 is wired up correctly.
//! - saya-tee and Piltover agree on the **state-diff serialization format**: saya-tee embeds a
//!   Poseidon commitment over the extracted diff as `report_data`; Piltover's `validate_input`
//!   recomputes the same commitment from the settlement calldata and requires a byte-identical
//!   match. A serialization drift anywhere in the chain (saya-tee's diff extractor, Piltover's
//!   decoder, the commitment schema) fails this check.
//! - Settlement advances **block-by-block**: after each driven L3 block, Piltover's `block_number`
//!   matches L3's tip and `state_root` / `block_hash` transition to non-zero values.
//!
//! ## What this does NOT prove
//!
//! - Real TEE attestation (AMD hardware signing, quote freshness, VCEK chain validity).
//! - SP1 proof soundness or on-chain SP1 verification.
//! - Binding of attestations to a specific enclave/instance.
//!
//! ## Required binaries
//!
//! Both are built from [dojoengine/saya](https://github.com/dojoengine/saya) at rev `5a3b8c9`.
//!
//! - `saya-ops`: discovered via `SAYA_OPS_BIN` or `$PATH`.
//! - `saya-tee`: discovered via `SAYA_TEE_BIN` or `$PATH`.

use std::time::Duration;

use anyhow::Result;

mod assertions;
mod bootstrap;
mod nodes;
mod saya;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    init_logging();

    println!("=== saya-tee e2e test starting ===");

    // 1. Spawn L2 dev Katana in-process.
    let l2 = nodes::spawn_l2().await;
    println!("L2 Katana ready at {}", l2.url());

    // 2. Bootstrap mock TEE registry + Piltover on L2 via saya-ops.
    let bootstrap = bootstrap::bootstrap_l2(&l2).await?;
    println!(
        "L2 contracts deployed: piltover={} tee_registry={}",
        hex_felt(&bootstrap.piltover_address),
        hex_felt(&bootstrap.tee_registry_address)
    );

    // 3. Spawn L3 rollup Katana with TEE config + settlement pointed at L2.
    let l3 = nodes::spawn_l3(&l2, bootstrap.piltover_address).await;
    println!("L3 Katana ready at {}", l3.url());

    // 4. Spawn saya-tee --mock-prove as a sidecar (RAII guard kills on drop).
    let _saya = saya::spawn_saya_tee(&saya::SayaTeeConfig {
        rollup_rpc: l3.url(),
        settlement_rpc: l2.url(),
        piltover_address: bootstrap.piltover_address,
        tee_registry_address: bootstrap.tee_registry_address,
        settlement_account_address: bootstrap.account_address,
        settlement_account_private_key: bootstrap.account_private_key,
    })?;
    println!("saya-tee sidecar spawned");

    // 5. Sanity-check Piltover's initial state: state_root and block_hash must be zero and
    //    block_number must be the Felt::MAX sentinel. Proves bootstrap produced a clean-slate
    //    settlement contract and that the saya-tee sidecar hasn't pushed anything prematurely.
    assertions::assert_initial_state(&l2, bootstrap.piltover_address).await?;

    // 6. Drive L3 one block at a time and assert Piltover settles each block before driving the
    //    next. Catches regressions a bulk-then-settle flow would miss: nonce drift across
    //    iterations, saya-tee batching the wrong ranges, stateful proof-pipeline bugs that only
    //    surface on the second or third update.
    const N_BLOCKS: usize = 3;
    for i in 1..=N_BLOCKS {
        println!("--- iteration {i}/{N_BLOCKS} ---");

        nodes::drive_l3_block(&l3).await?;

        println!("Drove L3 block, waiting for Piltover to settle");

        assertions::wait_for_settlement(
            &l2,
            &l3,
            bootstrap.piltover_address,
            Duration::from_secs(180),
        )
        .await?;
    }

    println!("=== saya-tee e2e test PASSED ===");
    Ok(())
}

/// Configures a tracing subscriber so logs emitted by Katana and saya-tee (which
/// both use `tracing` internally) surface to the terminal. The test itself uses
/// plain `println!` / `eprintln!`.
fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,saya_tee_e2e_test=debug,katana_node=warn,katana_core=warn")
    });
    if let Err(e) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("failed to init tracing subscriber: {e}");
    }
}

fn hex_felt(felt: &starknet_types_core::felt::Felt) -> String {
    format!("0x{:x}", felt)
}
