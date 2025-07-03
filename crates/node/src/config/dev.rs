use katana_gas_oracle::{
    DEFAULT_ETH_L1_DATA_GAS_PRICE, DEFAULT_ETH_L1_GAS_PRICE, DEFAULT_ETH_L2_GAS_PRICE,
    DEFAULT_STRK_L1_DATA_GAS_PRICE, DEFAULT_STRK_L1_GAS_PRICE, DEFAULT_STRK_L2_GAS_PRICE,
};
use katana_primitives::block::GasPrices;

/// Development configuration.
#[derive(Debug, Clone)]
pub struct DevConfig {
    /// Whether to enable paying fees for transactions.
    ///
    /// If disabled, the transaction's sender will not be charged for the transaction. Any fee
    /// related checks will be skipped.
    ///
    /// For example, if the transaction's fee resources (ie max fee) is higher than the sender's
    /// balance, the transaction will still be considered valid.
    pub fee: bool,

    /// Whether to enable account validation when sending transaction.
    ///
    /// If disabled, the transaction's sender validation logic will not be executed in any
    /// circumstances. Sending a transaction with invalid signatures, will be considered valid.
    ///
    /// In the case where fee estimation or transaction simulation is done *WITHOUT* the
    /// `SKIP_VALIDATE` flag, if validation is disabled, then it would be as if the
    /// estimation/simulation was sent with `SKIP_VALIDATE`. Using `SKIP_VALIDATE` while
    /// validation is disabled is a no-op.
    pub account_validation: bool,

    /// Fixed L1 gas prices for development.
    ///
    /// These are the prices that will be used for calculating the gas fee for transactions.
    pub fixed_gas_prices: Option<FixedL1GasPriceConfig>,
}

// TODO: move to gas oracle options
/// Fixed gas prices for development.
#[derive(Debug, Clone)]
pub struct FixedL1GasPriceConfig {
    pub l2_gas_prices: GasPrices,
    pub l1_gas_prices: GasPrices,
    pub l1_data_gas_prices: GasPrices,
}

impl std::default::Default for FixedL1GasPriceConfig {
    fn default() -> Self {
        Self {
            l2_gas_prices: GasPrices::new(DEFAULT_ETH_L2_GAS_PRICE, DEFAULT_STRK_L2_GAS_PRICE),
            l1_gas_prices: GasPrices::new(DEFAULT_ETH_L1_GAS_PRICE, DEFAULT_STRK_L1_GAS_PRICE),
            l1_data_gas_prices: GasPrices::new(
                DEFAULT_ETH_L1_DATA_GAS_PRICE,
                DEFAULT_STRK_L1_DATA_GAS_PRICE,
            ),
        }
    }
}

impl std::default::Default for DevConfig {
    fn default() -> Self {
        Self { fee: true, account_validation: true, fixed_gas_prices: None }
    }
}
