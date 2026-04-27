pub mod env;

use std::time::Duration;

use anyhow::Result;
use piltover::{AppchainContractReader, ProgramInfo};
use starknet::{
    core::types::{
        BlockId, BlockTag, Felt, MaybePreConfirmedBlockWithTxs, ReceiptBlock, Transaction,
    },
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
};

/// Decoded appchain state returned by the piltover `get_state` entry point.
#[derive(Debug)]
pub struct AppchainState {
    pub state_root: Felt,
    pub block_number: Felt,
    pub block_hash: Felt,
}

/// Query the piltover contract for the current settled chain state.
pub async fn get_settlement_state(
    provider: &JsonRpcClient<HttpTransport>,
    piltover_address: Felt,
) -> Result<AppchainState> {
    let (state_root, block_number, block_hash) =
        AppchainContractReader::new(piltover_address, provider)
            .with_block(BlockId::Tag(BlockTag::Latest))
            .get_state()
            .call()
            .await?;

    Ok(AppchainState {
        state_root,
        block_number,
        block_hash,
    })
}

/// Query the piltover contract for the configured program info.
pub async fn get_program_info(
    provider: &JsonRpcClient<HttpTransport>,
    piltover_address: Felt,
) -> Result<ProgramInfo> {
    Ok(AppchainContractReader::new(piltover_address, provider)
        .with_block(BlockId::Tag(BlockTag::Latest))
        .get_program_info()
        .call()
        .await?)
}

/// Query the piltover contract for the configured fact registry address.
pub async fn get_facts_registry(
    provider: &JsonRpcClient<HttpTransport>,
    piltover_address: Felt,
) -> Result<Felt> {
    let address = AppchainContractReader::new(piltover_address, provider)
        .with_block(BlockId::Tag(BlockTag::Latest))
        .get_facts_registry()
        .call()
        .await?;

    Ok(Felt::from(address))
}

/// Poll the piltover `get_state` entry point until `block_number >= target_block`
/// or the deadline is exceeded.
///
/// The initial (genesis) state uses `Felt::MAX` as a sentinel meaning no block has
/// been settled yet. This function treats that as "not yet at target".
pub async fn wait_for_settlement(
    provider: &JsonRpcClient<HttpTransport>,
    piltover_address: Felt,
    target_block: u64,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<AppchainState> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let state = get_settlement_state(provider, piltover_address).await?;

        // Felt::MAX is the genesis sentinel — no block settled yet.
        if state.block_number != Felt::MAX {
            let settled: u64 = state.block_number.try_into()?;
            if settled >= target_block {
                return Ok(state);
            }
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timeout waiting for settlement of block {}", target_block);
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Poll `get_transaction_receipt` until the tx is included in a confirmed block,
/// then return that block number.
///
/// Use this after `execute_v3(...).send().await` to get the actual block number
/// containing the transaction — `block_number()` alone can return a stale value
/// if the tx hasn't been mined yet.
pub async fn wait_for_tx_block(
    provider: &JsonRpcClient<HttpTransport>,
    tx_hash: Felt,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<u64> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if let Ok(receipt) = provider.get_transaction_receipt(tx_hash).await {
            match receipt.block {
                ReceiptBlock::Block { block_number, .. } => return Ok(block_number),
                ReceiptBlock::PreConfirmed { .. } => {}
            }
        } else {
            // Handle the error case if needed
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timeout waiting for tx {tx_hash:#x} to be included in a block");
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Build a JSON-RPC provider from a URL string.
pub fn provider(url: &str) -> JsonRpcClient<HttpTransport> {
    JsonRpcClient::new(HttpTransport::new(
        url.parse::<url::Url>().expect("invalid RPC URL"),
    ))
}

/// Walk up from `CARGO_MANIFEST_DIR` until `compose.yml` is found.
pub fn repo_root() -> std::path::PathBuf {
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !dir.join("compose.yml").exists() {
        dir = dir
            .parent()
            .expect("reached filesystem root without finding compose.yml")
            .to_path_buf();
    }
    dir
}

/// Start the full compose stack and block until all healthchecks pass.
/// Tears down any existing stack (including volumes) first to ensure a clean state.
pub fn compose_up() {
    compose_down();
    let status = std::process::Command::new("docker")
        .args(["compose", "up", "--build", "--wait"])
        .current_dir(repo_root())
        .status()
        .expect("failed to run docker compose up");
    assert!(status.success(), "docker compose up --wait failed");
}

/// Tear down the compose stack and remove volumes.
pub fn compose_down() {
    let _ = std::process::Command::new("docker")
        .args(["compose", "down", "-v"])
        .current_dir(repo_root())
        .status();
}

/// RAII guard that calls [`compose_down`] when dropped.
///
/// Ensures the compose stack is torn down even if the test panics,
/// which is important for CI where there is no manual cleanup.
pub struct ComposeGuard;

impl Drop for ComposeGuard {
    fn drop(&mut self) {
        compose_down();
    }
}

/// Poll L3 blocks starting from `from_block` until an L1Handler transaction
/// with the given `entry_point_selector` appears, then return its hash.
pub async fn wait_for_l1_handler(
    provider: &JsonRpcClient<HttpTransport>,
    entry_point_selector: Felt,
    from_block: u64,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<Felt> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut current = from_block;

    loop {
        match provider.get_block_with_txs(BlockId::Number(current)).await {
            Ok(block) => {
                let txs = match block {
                    MaybePreConfirmedBlockWithTxs::Block(b) => b.transactions,
                    MaybePreConfirmedBlockWithTxs::PreConfirmedBlock(b) => b.transactions,
                };
                for tx in txs {
                    if let Transaction::L1Handler(l1tx) = tx {
                        if l1tx.entry_point_selector == entry_point_selector {
                            return Ok(l1tx.transaction_hash);
                        }
                    }
                }
                current += 1;
            }
            Err(_) => {
                // Block not yet mined; check deadline then wait.
                if tokio::time::Instant::now() >= deadline {
                    anyhow::bail!(
                        "timeout waiting for L1Handler with selector {entry_point_selector:#x} \
                         from block {from_block}"
                    );
                }
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}
