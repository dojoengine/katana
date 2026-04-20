//! Piltover state assertions. See `main.rs` for the overall test scope and the real/mock split.
//!
//! Each call to [`wait_for_settlement`] exercises the full settlement round-trip: saya-tee polls
//! L3 (`starknet_blockNumber`, `starknet_getStateUpdate`), fetches the mock TEE quote from L3's
//! `tee_generateQuote`, synthesizes a stub `OnchainProof`, submits `update_state` to Piltover on
//! L2, which runs its real `validate_input` (Poseidon commitment recompute, byte-matched against
//! the mock quote's `report_data`) before advancing state. Only the cryptographic steps
//! (AMD signing, SP1 proving, on-chain SP1 verify) are bypassed.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cainome::rs::abigen;
use starknet::providers::Provider;
use starknet_types_core::felt::Felt;

use crate::nodes::{L2InProcess, L3InProcess};

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

/// Asserts Piltover is in its freshly-deployed genesis state: `state_root` and
/// `block_hash` are both zero and `block_number` is the `Felt::MAX` sentinel,
/// meaning no L3 blocks have been settled yet.
pub async fn assert_initial_state(l2: &L2InProcess, piltover_address: Felt) -> Result<()> {
    let provider = l2.provider();
    let core_contract = CoreContractReader::new(piltover_address, &provider);

    let (state_root, block_number, block_hash) = core_contract
        .get_state()
        .call()
        .await
        .context("failed to call Piltover get_state at initial state check")?;

    println!(
        "Piltover initial state: block_number={} state_root={} block_hash={}",
        hex(&block_number),
        hex(&state_root),
        hex(&block_hash)
    );

    if block_number != Felt::MAX {
        return Err(anyhow!(
            "expected Piltover block_number == Felt::MAX at genesis, got {}",
            hex(&block_number)
        ));
    }
    if state_root != Felt::ZERO {
        return Err(anyhow!(
            "expected Piltover state_root == 0 at genesis, got {}",
            hex(&state_root)
        ));
    }
    if block_hash != Felt::ZERO {
        return Err(anyhow!(
            "expected Piltover block_hash == 0 at genesis, got {}",
            hex(&block_hash)
        ));
    }

    Ok(())
}

/// Polls Piltover's `get_state()` until `block_number` matches the L3's current tip,
/// and then asserts `state_root` and `block_hash` are both non-zero (i.e. genuine
/// post-settlement values, not a zero-pin regression).
///
/// The L3 tip is sampled once at the start — the driver has stopped submitting
/// transactions by the time this is called, so the rollup's tip is stable.
pub async fn wait_for_settlement(
    l2: &L2InProcess,
    l3: &L3InProcess,
    piltover_address: Felt,
    timeout: Duration,
) -> Result<()> {
    let l3_tip =
        l3.provider().block_number().await.context("failed to fetch L3 latest block number")?;

    let expected_block_number = Felt::from(l3_tip);
    println!("Waiting for Piltover to settle up to L3 block {l3_tip}");

    let provider = l2.provider();
    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for Piltover block_number to reach {l3_tip} after {timeout:?}"
            ));
        }

        let core_contract = CoreContractReader::new(piltover_address, &provider);
        match core_contract.get_state().call().await {
            // AppchainState layout: [state_root, block_number, block_hash]
            Ok((state_root, block_number, block_hash)) => {
                if block_number != expected_block_number {
                    eprintln!(
                        "[debug] Piltover at block_number={}, waiting for {l3_tip}",
                        hex(&block_number)
                    );
                } else {
                    if state_root == Felt::ZERO {
                        return Err(anyhow!(
                            "Piltover block_number reached {l3_tip} but state_root is zero — \
                             expected a non-zero Poseidon commitment"
                        ));
                    }
                    if block_hash == Felt::ZERO {
                        return Err(anyhow!(
                            "Piltover block_number reached {l3_tip} but block_hash is zero"
                        ));
                    }
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
