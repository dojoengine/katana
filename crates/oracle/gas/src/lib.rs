use std::fmt::Debug;
use std::future::Future;

use katana_primitives::block::GasPrices;
use url::Url;

mod fixed;
mod sampled;

pub use fixed::{
    FixedPriceOracle, DEFAULT_ETH_L1_DATA_GAS_PRICE, DEFAULT_ETH_L1_GAS_PRICE,
    DEFAULT_ETH_L2_GAS_PRICE, DEFAULT_STRK_L1_DATA_GAS_PRICE, DEFAULT_STRK_L1_GAS_PRICE,
    DEFAULT_STRK_L2_GAS_PRICE,
};
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
        let sampler = sampled::Sampler::ethereum(url);
        let oracle = sampled::SampledPriceOracle::new(sampler);
        Self::Sampled(oracle)
    }

    /// Creates a new gas oracle that samples the gas prices from a Starknet chain.
    pub fn sampled_starknet(url: Url) -> Self {
        let sampler = sampled::Sampler::starknet(url);
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
