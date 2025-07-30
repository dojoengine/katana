use std::fmt::Debug;

use katana_primitives::block::GasPrices;
use num_traits::ToPrimitive;
use starknet::core::types::{BlockId, BlockTag, MaybePreConfirmedBlockWithTxHashes};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;

use super::SampledPrices;

#[derive(Debug, Clone)]
pub struct StarknetSampler<P = JsonRpcClient<HttpTransport>> {
    provider: P,
}

impl<P> StarknetSampler<P> {
    /// Creates a new [`StarknetGasPriceSampler`] using the given provider.
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P: starknet::providers::Provider> StarknetSampler<P> {
    pub async fn sample(&self) -> anyhow::Result<SampledPrices> {
        let block_id = BlockId::Tag(BlockTag::Latest);
        let block = self.provider.get_block_with_tx_hashes(block_id).await?;

        let (l1_gas_price, l2_gas_price, l1_data_gas_price) = match block {
            MaybePreConfirmedBlockWithTxHashes::Block(block) => {
                (block.l1_gas_price, block.l2_gas_price, block.l1_data_gas_price)
            }
            MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(pending) => {
                (pending.l1_gas_price, pending.l2_gas_price, pending.l1_data_gas_price)
            }
        };

        let l2_gas_prices = GasPrices::new(
            l2_gas_price.price_in_wei.to_u128().unwrap().try_into()?,
            l2_gas_price.price_in_fri.to_u128().unwrap().try_into()?,
        );

        let l1_gas_prices = GasPrices::new(
            l1_gas_price.price_in_wei.to_u128().unwrap().try_into()?,
            l1_gas_price.price_in_fri.to_u128().unwrap().try_into()?,
        );

        let l1_data_gas_prices = GasPrices::new(
            l1_data_gas_price.price_in_wei.to_u128().unwrap().try_into()?,
            l1_data_gas_price.price_in_fri.to_u128().unwrap().try_into()?,
        );

        Ok(SampledPrices { l2_gas_prices, l1_gas_prices, l1_data_gas_prices })
    }
}
