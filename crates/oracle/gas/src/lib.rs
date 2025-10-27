use std::fmt::Debug;
use std::future::Future;

use alloy_provider::network;
use katana_primitives::block::GasPrices;
use starknet::providers::jsonrpc::HttpTransport;
use url::Url;

mod fixed;
mod sampled;

pub use fixed::{
    FixedPriceOracle, DEFAULT_ETH_L1_DATA_GAS_PRICE, DEFAULT_ETH_L1_GAS_PRICE,
    DEFAULT_ETH_L2_GAS_PRICE, DEFAULT_STRK_L1_DATA_GAS_PRICE, DEFAULT_STRK_L1_GAS_PRICE,
    DEFAULT_STRK_L2_GAS_PRICE,
};
pub use sampled::ethereum::EthSampler;
pub use sampled::starknet::StarknetSampler;
pub use sampled::{SampledPriceOracle, Sampler};

#[derive(Debug)]
pub enum GasPriceOracle {
    Fixed(fixed::FixedPriceOracle),
    Sampled(sampled::SampledPriceOracle),
}

impl GasPriceOracle {
    pub fn fixed(
        l2_gas_prices: GasPrices,
        l1_gas_prices: GasPrices,
        l1_data_gas_prices: GasPrices,
    ) -> Self {
        GasPriceOracle::Fixed(fixed::FixedPriceOracle::new(
            l2_gas_prices,
            l1_gas_prices,
            l1_data_gas_prices,
        ))
    }

    /// Creates a new gas oracle that samples the gas prices from an Ethereum chain.
    pub fn sampled_ethereum(url: Url) -> Self {
        let provider = alloy_provider::RootProvider::<network::Ethereum>::new_http(url);
        let sampler = sampled::ethereum::EthSampler::new(provider);
        let oracle = sampled::SampledPriceOracle::new(sampler);
        Self::Sampled(oracle)
    }

    /// Creates a new gas oracle that samples the gas prices from a Starknet chain.
    pub fn sampled_starknet(url: Url) -> Self {
        let provider = starknet::providers::JsonRpcClient::new(HttpTransport::new(url));
        let sampler = sampled::starknet::StarknetSampler::new(provider);
        let oracle = sampled::SampledPriceOracle::new(sampler);
        Self::Sampled(oracle)
    }

    /// Returns the current L1 gas prices.
    pub fn l1_gas_prices(&self) -> GasPrices {
        match self {
            GasPriceOracle::Fixed(fixed) => fixed.l1_gas_prices().clone(),
            GasPriceOracle::Sampled(sampled) => sampled.avg_l1_gas_prices(),
        }
    }

    /// Returns the current data gas prices.
    pub fn l1_data_gas_prices(&self) -> GasPrices {
        match self {
            GasPriceOracle::Fixed(fixed) => fixed.l1_data_gas_prices().clone(),
            GasPriceOracle::Sampled(sampled) => sampled.avg_l1_data_gas_prices(),
        }
    }

    /// Returns the current L2 gas prices.
    pub fn l2_gas_prices(&self) -> GasPrices {
        match self {
            GasPriceOracle::Fixed(fixed) => fixed.l2_gas_prices().clone(),
            GasPriceOracle::Sampled(sampled) => sampled.avg_l2_gas_prices(),
        }
    }

    pub fn run_worker(&self) -> Option<impl Future<Output = ()> + 'static> {
        match self {
            Self::Fixed(..) => None,
            Self::Sampled(sampled) => Some(sampled.run_worker()),
        }
    }

    pub fn create_for_testing() -> Self {
        GasPriceOracle::Fixed(fixed::FixedPriceOracle::default())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use katana_primitives::block::GasPrices;

    use crate::GasPriceOracle;

    #[test]
    fn fixed_gpo() {
        const L2_GAS_PRICES: GasPrices = unsafe { GasPrices::new_unchecked(101, 102) };
        const L1_GAS_PRICES: GasPrices = unsafe { GasPrices::new_unchecked(201, 202) };
        const L1_DATA_GAS_PRICES: GasPrices = unsafe { GasPrices::new_unchecked(301, 302) };

        let gpo = GasPriceOracle::fixed(L2_GAS_PRICES, L1_GAS_PRICES, L1_DATA_GAS_PRICES);

        assert_eq!(gpo.l2_gas_prices(), L2_GAS_PRICES);
        assert_eq!(gpo.l1_gas_prices(), L1_GAS_PRICES);
        assert_eq!(gpo.l1_data_gas_prices(), L1_DATA_GAS_PRICES);

        // Sleep for s few seconds to ensure the gas prices don't change.
        std::thread::sleep(Duration::from_secs(3));

        assert_eq!(gpo.l2_gas_prices(), L2_GAS_PRICES);
        assert_eq!(gpo.l1_gas_prices(), L1_GAS_PRICES);
        assert_eq!(gpo.l1_data_gas_prices(), L1_DATA_GAS_PRICES);
    }
}
