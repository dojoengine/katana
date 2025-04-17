#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceBounds {
    /// The max amount of the resource that can be used in the tx
    pub max_amount: u64,
    /// The max price per unit of this resource for this tx
    pub max_price_per_unit: u128,
}

// Aliased to match the feeder gateway API
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceBoundsMapping {
    #[serde(alias = "L1_GAS")]
    pub l1_gas: ResourceBounds,
    #[serde(alias = "L2_GAS")]
    pub l2_gas: ResourceBounds,
    #[serde(alias = "L1_DATA_GAS")]
    pub l1_data_gas: ResourceBounds,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PriceUnit {
    #[serde(rename = "WEI")]
    Wei,
    #[default]
    #[serde(rename = "FRI")]
    Fri,
}

/// Information regarding the fee and gas usages of a transaction.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeeInfo {
    /// The gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    pub l1_gas_price: u128,
    /// The L2 gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    pub l2_gas_price: u128,
    /// The data gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    pub l1_data_gas_price: u128,
    /// The estimated fee for the transaction (in wei or fri, depending on the tx version), equals
    /// to gas_consumed*gas_price + data_gas_consumed*data_gas_price
    pub overall_fee: u128,
    /// Units in which the fee is given
    pub unit: PriceUnit,
}
