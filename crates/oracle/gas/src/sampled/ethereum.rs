use alloy_provider::{network, Provider};
use futures::future::BoxFuture;
use katana_primitives::block::{GasPrice, GasPrices};

use crate::sampled::SampledPrices;
use crate::Sampler;

pub type EthereumSampler = alloy_provider::RootProvider<network::Ethereum>;

impl Sampler for EthereumSampler {
    fn sample(&self) -> BoxFuture<'_, anyhow::Result<SampledPrices>> {
        Box::pin(async {
            let block = self.get_block_number().await?;
            let fee_history = self.get_fee_history(1, block.into(), &[]).await?;

            let l1_gas_prices = {
                let latest_gas_price = fee_history.base_fee_per_gas.last().unwrap();
                let eth_price = GasPrice::try_from(*latest_gas_price)?;
                let strk_price = eth_price; // TODO: Implement STRK price calculation from L1
                GasPrices::new(eth_price, strk_price)
            };

            let l1_data_gas_prices = {
                let blob_fee_history = fee_history.base_fee_per_blob_gas;
                let avg_blob_base_fee = blob_fee_history.iter().last().unwrap();
                let eth_price = GasPrice::try_from(*avg_blob_base_fee)?;
                let strk_price = eth_price; // TODO: Implement STRK price calculation from L1
                GasPrices::new(eth_price, strk_price)
            };

            let l2_gas_prices = l1_gas_prices.clone();

            Ok(SampledPrices { l2_gas_prices, l1_gas_prices, l1_data_gas_prices })
        })
    }
}
