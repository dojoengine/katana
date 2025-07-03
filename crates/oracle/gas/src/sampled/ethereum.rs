use std::fmt::Debug;

use alloy_provider::RootProvider;
use alloy_transport_http::Http;
use katana_primitives::block::{GasPrice, GasPrices};
use reqwest::Client;

use super::SampledPrices;

#[derive(Debug, Clone)]
pub struct EthSampler<P = RootProvider<Http<Client>>> {
    provider: P,
}

impl<P> EthSampler<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P: alloy_provider::Provider<Http<Client>>> EthSampler<P> {
    pub async fn sample(&self) -> anyhow::Result<SampledPrices> {
        let block = self.provider.get_block_number().await?;
        let fee_history = self.provider.get_fee_history(1, block.into(), &[]).await?;

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
    }
}
