#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
pub struct ResourceBounds {
    /// The max amount of the resource that can be used in the tx
    pub max_amount: u64,
    /// The max price per unit of this resource for this tx
    pub max_price_per_unit: u128,
}

impl ResourceBounds {
    pub const ZERO: Self = Self { max_amount: 0, max_price_per_unit: 0 };
}

// Aliased to match the feeder gateway API
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AllResourceBoundsMapping {
    /// L1 gas bounds - covers L2→L1 messages sent by the transaction
    #[serde(alias = "L1_GAS")]
    pub l1_gas: ResourceBounds,
    /// L2 gas bounds - covers L2 resources including computation, tx payload, event emission, code
    /// size, etc. Units: 1 Cairo step = 100 L2 gas
    #[serde(alias = "L2_GAS")]
    pub l2_gas: ResourceBounds,
    /// L1 data gas (blob gas) bounds - covers the cost of submitting state diffs as blobs on L1
    #[serde(alias = "L1_DATA_GAS")]
    pub l1_data_gas: ResourceBounds,
}

/// Transaction resource bounds.
///
/// ## NOTE
///
/// As of Starknet v0.14.0, only transactions with all three bounds (L1 gas, L2 gas, L1 data gas)
/// are accepted by the sequencer. Transactions with only L1 gas bounds are supported for
/// backward compatibility but will be rejected in v0.14.0+.
///
/// For further details, refer to [Starknet v0.13.4 pre-release notes](https://community.starknet.io/t/starknet-v0-13-4-pre-release-notes/115257).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResourceBoundsMapping {
    /// Legacy bounds; only L1 gas bounds specified (backward compatibility).
    ///
    /// Raw resources are converted to L1 gas for cost calculation. Prior to 0.14.0, the L2 gas
    /// bounds is signed and present in the but is always hardcoded to be zero thus, the L2 gas
    /// field is completely ommitted from this variant and is assumed to be zero during
    /// transaction hash computation.
    ///
    /// Supported in Starknet v0.13.4 but rejected in v0.14.0+.
    L1Gas(ResourceBounds),

    /// All three resource bounds specified: L1 gas, L2 gas, and L1 data gas.
    ///
    /// The required format as of Starknet v0.14.0.
    All(AllResourceBoundsMapping),
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

#[cfg(feature = "serde")]
mod _serde {
    use super::ResourceBounds;
    use crate::utils::serde::{
        deserialize_hex_u128, deserialize_hex_u64, serialize_hex_u128, serialize_hex_u64,
    };

    #[derive(::serde::Serialize, ::serde::Deserialize)]
    struct HumanReadableResourceBounds {
        #[serde(serialize_with = "serialize_hex_u64", deserialize_with = "deserialize_hex_u64")]
        max_amount: u64,
        #[serde(serialize_with = "serialize_hex_u128", deserialize_with = "deserialize_hex_u128")]
        max_price_per_unit: u128,
    }

    #[derive(::serde::Serialize, ::serde::Deserialize)]
    struct NonHumanReadableResourceBounds {
        max_amount: u64,
        max_price_per_unit: u128,
    }

    impl ::serde::Serialize for ResourceBounds {
        fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            if serializer.is_human_readable() {
                let helper = HumanReadableResourceBounds {
                    max_amount: self.max_amount,
                    max_price_per_unit: self.max_price_per_unit,
                };

                helper.serialize(serializer)
            } else {
                let helper = NonHumanReadableResourceBounds {
                    max_amount: self.max_amount,
                    max_price_per_unit: self.max_price_per_unit,
                };

                helper.serialize(serializer)
            }
        }
    }

    impl<'de> ::serde::Deserialize<'de> for ResourceBounds {
        fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            if deserializer.is_human_readable() {
                let helper = HumanReadableResourceBounds::deserialize(deserializer)?;
                Ok(ResourceBounds {
                    max_amount: helper.max_amount,
                    max_price_per_unit: helper.max_price_per_unit,
                })
            } else {
                let helper = NonHumanReadableResourceBounds::deserialize(deserializer)?;
                Ok(ResourceBounds {
                    max_amount: helper.max_amount,
                    max_price_per_unit: helper.max_price_per_unit,
                })
            }
        }
    }
}
