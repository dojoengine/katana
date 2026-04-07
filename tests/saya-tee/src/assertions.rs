//! Polling assertions for the saya-tee e2e test.
//!
//! The single assertion that proves the entire pipeline worked is:
//!
//! > Piltover's `get_state()` returns a `block_number` that is no longer the
//! > genesis sentinel `Felt::MAX`.
//!
//! This means saya-tee successfully:
//!
//! 1. Polled L3 for new blocks via `starknet_blockNumber` / `starknet_getStateUpdate`.
//! 2. Fetched a TEE attestation from L3 via `tee_generateQuote` (served by
//!    `katana_tee::MockProvider`).
//! 3. Synthesized a fake `OnchainProof` via the `--mock-prove` fast path.
//! 4. Submitted `update_state` to Piltover with that proof.
//! 5. Piltover dispatched `verify_sp1_proof` to the mock TEE registry which returned the journal
//!    verbatim.
//! 6. Piltover's `validate_input` recomputed the Poseidon commitment and matched it against the
//!    report_data the mock prover had embedded.
//! 7. Piltover advanced its on-chain state.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use starknet::core::types::{BlockId, BlockTag, FunctionCall};
use starknet::macros::selector;
use starknet::providers::Provider;
use starknet_types_core::felt::Felt;
use tracing::{debug, info};

use crate::nodes::L2Subprocess;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Polls Piltover's `get_state()` until `block_number != Felt::MAX`, or
/// returns an error after `timeout`.
pub async fn wait_for_settlement(
    l2: &L2Subprocess,
    piltover_address: Felt,
    timeout: Duration,
) -> Result<()> {
    let provider = l2.provider();
    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for Piltover state to advance past Felt::MAX after {timeout:?}"
            ));
        }

        match provider
            .call(
                FunctionCall {
                    contract_address: piltover_address,
                    entry_point_selector: selector!("get_state"),
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await
        {
            Ok(state) => {
                // AppchainState layout: [state_root, block_number, block_hash]
                let block_number = state
                    .get(1)
                    .copied()
                    .context("Piltover get_state returned fewer than 2 felts")?;

                if block_number == Felt::MAX {
                    debug!("Piltover still at genesis sentinel (block_number = Felt::MAX)");
                } else {
                    info!(
                        block_number = %hex(&block_number),
                        state_root = %hex(state.first().unwrap_or(&Felt::ZERO)),
                        "Piltover state advanced"
                    );
                    return Ok(());
                }
            }
            Err(e) => {
                debug!(error = %e, "Piltover get_state call failed, retrying");
            }
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn hex(felt: &Felt) -> String {
    format!("0x{:x}", felt)
}
