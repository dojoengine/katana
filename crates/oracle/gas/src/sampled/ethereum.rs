use std::fmt::Debug;

use alloy_provider::network;
use katana_primitives::block::{GasPrice, GasPrices};

use super::SampledPrices;

/// Type alias for an Ethereum network provider with alloy's recommended default fillers.
type EthereumProvider = alloy_provider::RootProvider<network::Ethereum>;

#[derive(Debug, Clone)]
pub struct EthSampler<P = EthereumProvider> {
    provider: P,
}

impl<P> EthSampler<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

// The alloy Provider trait is only implemented for the Ethereum network provider.
impl<P: alloy_provider::Provider<network::Ethereum>> EthSampler<P> {
    async fn sample_prices(&self) -> anyhow::Result<SampledPrices> {
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

impl<P> super::Sampler for EthSampler<P>
where
    P: alloy_provider::Provider<network::Ethereum> + Send + Sync,
{
    async fn sample(&self) -> anyhow::Result<SampledPrices> {
        self.sample_prices().await
    }
}
