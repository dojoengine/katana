//! End-to-end test for Katana's embedded TEE settlement service, with all
//! cryptographic components swapped for permissive stubs.
//!
//! ## What's real
//!
//! - L2 dev Katana and L3 rollup Katana, both in-process.
//! - Piltover core contract + mock TEE registry on L2 (real Cairo, real state-transition math),
//!   deployed in-process from the class artifacts embedded in `katana-contracts`.
//! - L3's embedded settlement service: watches its own blocks, builds attestations, and submits
//!   `update_state(TeeInput)` to L2.
//! - The state-diff → Poseidon commitment → `report_data` → `validate_input` round-trip.
//!
//! ## What's mocked
//!
//! | Component | Real | Mock |
//! |-----------|------|------|
//! | Block attestation on L3 | AMD SEV-SNP hardware-signed quote | `katana_tee::MockAttester`: stub quote. `report_data` is a real Poseidon commitment over the state diff; only the hardware signature is absent. |
//! | SP1 proving | Real SP1 proof over the attestation | The mock attester forces the mock prover, which synthesizes a stub `VerifierJournal`. SP1 prover network is never contacted. |
//! | AMD KDS + cert-chain verification | Settlement walks AMD root → VCEK | Skipped in mock mode. |
//! | On-chain fact registry | Runs SP1 verifier in Cairo | `mock_amd_tee_registry`: returns the SP1 journal verbatim. |
//!
//! ## What this proves
//!
//! - End-to-end **plumbing** between L3, its embedded settlement service, Piltover, and L2 is wired
//!   up correctly.
//! - The settlement service and Piltover agree on the **state-diff serialization format**: the
//!   attestation embeds a Poseidon commitment over the state transition as `report_data`;
//!   Piltover's `validate_input` recomputes the same commitment from the settlement calldata and
//!   requires a byte-identical match. A serialization drift anywhere in the chain fails this check.
//! - Settlement advances **block-by-block**: after each driven L3 block, Piltover's `block_number`
//!   matches L3's tip and `state_root` / `block_hash` transition to non-zero values.
//! - Multi-block **backlog draining**: blocks driven in quick succession settle batch-by-batch.
//!
//! ## What this does NOT prove
//!
//! - Real TEE attestation (AMD hardware signing, quote freshness, VCEK chain validity).
//! - SP1 proof soundness or on-chain SP1 verification.
//! - Binding of attestations to a specific enclave/instance.

use std::time::Duration;

use anyhow::Result;

mod assertions;
mod bootstrap;
mod messaging;
mod nodes;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    init_logging();

    println!("=== saya-tee e2e test starting ===");

    // 1. Spawn L2 dev Katana in-process.
    let l2 = nodes::spawn_l2().await;
    println!("L2 Katana ready at {}", l2.url());

    // 2. Bootstrap mock TEE registry + Piltover on L2.
    let bootstrap = bootstrap::bootstrap_l2(&l2).await?;
    println!(
        "L2 contracts deployed: piltover={} tee_registry={}",
        hex_felt(&bootstrap.piltover_address),
        hex_felt(&bootstrap.tee_registry_address)
    );

    // 3. Spawn L3 rollup Katana with TEE config + the embedded settlement service pointed at L2.
    //    The service starts with the node — no sidecar process.
    let l3 = nodes::spawn_l3(&l2, &bootstrap).await;
    println!("L3 Katana ready at {} (embedded settlement service running)", l3.url());

    // 4. Sanity-check Piltover's initial state: state_root and block_hash must be zero and
    //    block_number must be the Felt::MAX sentinel. Proves bootstrap produced a clean-slate
    //    settlement contract and that the settlement service hasn't pushed anything prematurely.
    assertions::assert_initial_state(&l2, bootstrap.piltover_address).await?;

    // 5. Drive L3 one block at a time and assert Piltover settles each block before driving the
    //    next. Catches regressions a bulk-then-settle flow would miss: nonce drift across
    //    iterations, the service batching the wrong ranges, stateful proof-pipeline bugs that only
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

    // 5b. The embedded settlement service exposes its progress via `katana_settlementStatus`.
    //     Assert the RPC reflects the live settler: enabled, TEE prover, cursor caught up to the
    //     L3 tip, pointed at the real Piltover, no failures.
    assertions::assert_settlement_status(&l3).await?;

    // 6. Backlog drain: drive two blocks back-to-back without waiting in between, then wait for the
    //    service to catch up. Exercises the resume/backfill path (cursor behind head by more than
    //    one block) that the one-at-a-time loop above never hits.
    println!("--- backlog drain: 2 blocks back-to-back ---");
    nodes::drive_l3_block(&l3).await?;
    nodes::drive_l3_block(&l3).await?;
    assertions::wait_for_settlement(&l2, &l3, bootstrap.piltover_address, Duration::from_secs(180))
        .await?;

    // 7. Regression: drive a cross-chain message in each direction through a *settled* block. The
    //    loop above only settles plain transfer blocks, so it never builds a `messages_commitment`
    //    over a real message. A block that consumes an L1->L2 message only settles if the
    //    settlement service hashes L1->L2 messages with the Poseidon formula Katana commits to (not
    //    the Ethereum keccak formula); otherwise piltover's `update_state` rejects it with 'tee:
    //    invalid messages' and `wait_for_settlement` below times out.
    // Deploy the l1-handler contract on the appchain (declare blocks settle as
    // plain blocks before the message phases).
    let handler = messaging::deploy_msg_handler(&l3).await?;
    println!("Deployed appchain l1-handler contract at {}", hex_felt(&handler));

    println!("--- L1 -> L2 message, then settle ---");
    let tip_before = messaging::current_tip(&l3).await?;
    messaging::send_l1_to_l2(&l2, bootstrap.piltover_address, handler).await?;
    println!(
        "Sent send_message_to_appchain on L2; waiting for the appchain to relay the L1-handler"
    );
    messaging::wait_for_relay(&l3, tip_before, Duration::from_secs(30)).await?;
    assertions::wait_for_settlement(&l2, &l3, bootstrap.piltover_address, Duration::from_secs(180))
        .await?;

    println!("--- L2 -> L1 message, then settle ---");
    messaging::send_l2_to_l1(&l3, handler).await?;
    assertions::wait_for_settlement(&l2, &l3, bootstrap.piltover_address, Duration::from_secs(180))
        .await?;

    println!("=== saya-tee e2e test PASSED ===");
    Ok(())
}

/// Configures a tracing subscriber so logs emitted by Katana (including the
/// embedded settlement service) surface to the terminal. The test itself uses
/// plain `println!` / `eprintln!`.
fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,saya_tee_e2e_test=debug,settlement=debug,katana_node=warn,katana_core=warn",
        )
    });
    if let Err(e) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("failed to init tracing subscriber: {e}");
    }
}

fn hex_felt(felt: &starknet_types_core::felt::Felt) -> String {
    format!("0x{:x}", felt)
}
