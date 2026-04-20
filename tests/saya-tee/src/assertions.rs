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

use anyhow::{anyhow, Result};
use cainome::rs::abigen;
use starknet_types_core::felt::Felt;

use crate::nodes::L2InProcess;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

abigen!(CoreContract,
[
    {
        "type": "impl",
        "name": "StateImpl",
        "interface_name": "piltover::state::interface::IState"
    },
    {
        "type": "interface",
        "name": "piltover::state::interface::IState",
        "items": [
            {
                "type": "function",
                "name": "get_state",
                "inputs": [],
                "outputs": [
                    {
                        "type": "(core::felt252, core::felt252, core::felt252)"
                    }
                ],
                "state_mutability": "view"
            }
        ]
    }
]
);

/// Polls Piltover's `get_state()` until `block_number != Felt::MAX`, or
/// returns an error after `timeout`.
pub async fn wait_for_settlement(
    l2: &L2InProcess,
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

        let core_contract = CoreContractReader::new(piltover_address, &provider);
        match core_contract.get_state().call().await {
            // AppchainState layout: [state_root, block_number, block_hash]
            Ok((state_root, block_number, block_hash)) => {
                if block_number == Felt::MAX {
                    eprintln!("[debug] Piltover still at genesis sentinel (block_number = Felt::MAX)");
                } else {
                    println!(
                        "Piltover state advanced: block_number={} state_root={} block_hash={}",
                        hex(&block_number),
                        hex(&state_root),
                        hex(&block_hash)
                    );
                    return Ok(());
                }
            }
            Err(e) => {
                eprintln!("[debug] Piltover get_state call failed, retrying: {e}");
            }
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn hex(felt: &Felt) -> String {
    format!("0x{:x}", felt)
}
