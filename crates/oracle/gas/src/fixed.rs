use std::fmt::Debug;
use std::num::NonZeroU128;

use katana_primitives::block::{GasPrice, GasPrices};

#[derive(Debug)]
pub struct FixedPriceOracle {
    l2_gas_prices: GasPrices,
    l1_gas_prices: GasPrices,
    l1_data_gas_prices: GasPrices,
}

impl FixedPriceOracle {
    pub fn new(
        l2_gas_prices: GasPrices,
        l1_gas_prices: GasPrices,
        l1_data_gas_prices: GasPrices,
    ) -> Self {
        Self { l1_gas_prices, l2_gas_prices, l1_data_gas_prices }
    }

    pub fn l1_gas_prices(&self) -> &GasPrices {
        &self.l1_gas_prices
    }

    pub fn l2_gas_prices(&self) -> &GasPrices {
        &self.l2_gas_prices
    }

    pub fn l1_data_gas_prices(&self) -> &GasPrices {
        &self.l1_data_gas_prices
    }
}

impl Default for FixedPriceOracle {
    fn default() -> Self {
        Self {
            l1_gas_prices: GasPrices::new(DEFAULT_ETH_L1_GAS_PRICE, DEFAULT_STRK_L1_GAS_PRICE),
            l2_gas_prices: GasPrices::new(DEFAULT_ETH_L2_GAS_PRICE, DEFAULT_STRK_L2_GAS_PRICE),
            l1_data_gas_prices: GasPrices::new(
                DEFAULT_ETH_L1_DATA_GAS_PRICE,
                DEFAULT_STRK_L1_DATA_GAS_PRICE,
            ),
        }
    }
}

// Default l2 gas prices
pub const DEFAULT_ETH_L2_GAS_PRICE: GasPrice =
    GasPrice::new(NonZeroU128::new(20 * u128::pow(10, 9)).unwrap()); // Given in units of Wei.
pub const DEFAULT_STRK_L2_GAS_PRICE: GasPrice =
    GasPrice::new(NonZeroU128::new(20 * u128::pow(10, 9)).unwrap()); // Given in units of Fri.

// Default l1 gas prices
pub const DEFAULT_ETH_L1_GAS_PRICE: GasPrice =
    GasPrice::new(NonZeroU128::new(20 * u128::pow(10, 9)).unwrap()); // Given in units of Wei.
pub const DEFAULT_STRK_L1_GAS_PRICE: GasPrice =
    GasPrice::new(NonZeroU128::new(20 * u128::pow(10, 9)).unwrap()); // Given in units of Fri.

// Default l1 data gas prices
pub const DEFAULT_ETH_L1_DATA_GAS_PRICE: GasPrice =
    GasPrice::new(NonZeroU128::new(u128::pow(10, 6)).unwrap()); // Given in units of Wei.
pub const DEFAULT_STRK_L1_DATA_GAS_PRICE: GasPrice =
    GasPrice::new(NonZeroU128::new(u128::pow(10, 6)).unwrap()); // Given in units of Fri.
