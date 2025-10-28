use std::fmt::Debug;

use alloy_provider::network;
use futures::future::BoxFuture;
use katana_primitives::block::{GasPrice, GasPrices};

use crate::sampled::SampledPrices;
use crate::Sampler;

/// Type alias for an Ethereum network provider with alloy's recommended default fillers.
type RootEthereumProvider = alloy_provider::RootProvider<network::Ethereum>;

#[derive(Debug, Clone)]
pub struct EthereumSampler<P = RootEthereumProvider> {
    provider: P,
}

impl<P> EthereumSampler<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl EthereumSampler<RootEthereumProvider> {
    pub fn new_http(url: url::Url) -> Self {
        Self::new(RootEthereumProvider::new_http(url))
    }
}

impl<P> Sampler for EthereumSampler<P>
where
    P: alloy_provider::Provider<network::Ethereum> + Debug,
{
    fn sample(&self) -> BoxFuture<'_, anyhow::Result<SampledPrices>> {
        Box::pin(async {
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
        })
    }
}
