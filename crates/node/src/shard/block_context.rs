use std::sync::Arc;
use std::time::Duration;

use katana_chain_spec::ChainSpec;
use katana_primitives::block::{BlockIdOrTag, GasPrice, GasPrices};
use katana_primitives::env::BlockEnv;
use katana_primitives::Felt;
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_types::block::GetBlockWithTxHashesResponse;
use parking_lot::RwLock;
use tracing::{info, warn};

/// Background task that polls the base chain for new blocks and updates the shared `BlockEnv`.
#[derive(Debug, Clone)]
pub struct BlockContextListener {
    client: StarknetClient,
    block_env: Arc<RwLock<BlockEnv>>,
    #[allow(dead_code)]
    chain_spec: Arc<ChainSpec>,
    poll_interval: Duration,
}

impl BlockContextListener {
    pub fn new(
        client: StarknetClient,
        block_env: Arc<RwLock<BlockEnv>>,
        chain_spec: Arc<ChainSpec>,
        poll_interval: Duration,
    ) -> Self {
        Self { client, block_env, chain_spec, poll_interval }
    }

    pub async fn run(self) {
        info!(interval_secs = self.poll_interval.as_secs(), "Block context listener started.");

        loop {
            tokio::time::sleep(self.poll_interval).await;

            match self.client.get_block_with_tx_hashes(BlockIdOrTag::Latest).await {
                Ok(response) => {
                    if let Some(new_env) = self.extract_block_env(response) {
                        let block_number = new_env.number;
                        *self.block_env.write() = new_env;
                        info!(block_number, "Block context updated from base chain.");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to fetch block from base chain, will retry.");
                }
            }
        }
    }

    fn extract_block_env(&self, response: GetBlockWithTxHashesResponse) -> Option<BlockEnv> {
        match response {
            GetBlockWithTxHashesResponse::Block(block) => {
                let l1_gas_prices = GasPrices::new(
                    felt_to_gas_price(block.l1_gas_price.price_in_wei),
                    felt_to_gas_price(block.l1_gas_price.price_in_fri),
                );
                let l2_gas_prices = GasPrices::new(
                    felt_to_gas_price(block.l2_gas_price.price_in_wei),
                    felt_to_gas_price(block.l2_gas_price.price_in_fri),
                );
                let l1_data_gas_prices = GasPrices::new(
                    felt_to_gas_price(block.l1_data_gas_price.price_in_wei),
                    felt_to_gas_price(block.l1_data_gas_price.price_in_fri),
                );
                let starknet_version =
                    block.starknet_version.try_into().expect("invalid starknet version");

                Some(BlockEnv {
                    number: block.block_number,
                    timestamp: block.timestamp,
                    sequencer_address: block.sequencer_address,
                    l1_gas_prices,
                    l2_gas_prices,
                    l1_data_gas_prices,
                    starknet_version,
                })
            }
            GetBlockWithTxHashesResponse::PreConfirmed(_) => {
                // PreConfirmed blocks may not have all fields; skip them.
                None
            }
        }
    }
}

/// Convert a `Felt` gas price value to `GasPrice`, clamping zero to `GasPrice::MIN`.
fn felt_to_gas_price(felt: Felt) -> GasPrice {
    use std::num::NonZeroU128;
    let raw: u128 = felt.try_into().unwrap_or(1);
    match NonZeroU128::new(raw) {
        Some(v) => GasPrice::new(v),
        None => GasPrice::MIN,
    }
}
