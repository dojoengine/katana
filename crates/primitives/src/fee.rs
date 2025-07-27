#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", ::serde_with::serde_as, derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceBounds {
    /// The max amount of the resource that can be used in the tx
    #[serde_as(as = "NumAsHex")]
    pub max_amount: u64,
    /// The max price per unit of this resource for this tx
    #[serde_as(as = "NumAsHex")]
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
    /// bounds is signed and present in the but is always hardcoded to be zero thus, the L2 gas field is completely
    /// ommitted from this variant and is assumed to be zero during transaction hash computation.
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

use std::fmt::Formatter;

use serde::{de::Visitor, Deserializer};
use serde_with::{serde_as, DeserializeAs, SerializeAs};

pub struct NumAsHex;

struct NumAsHexVisitorU64;
struct NumAsHexVisitorU128;

impl SerializeAs<u64> for NumAsHex {
    fn serialize_as<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{value:#x}"))
    }
}

impl SerializeAs<&u64> for NumAsHex {
    fn serialize_as<S>(value: &&u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{value:#x}"))
    }
}

impl<'de> DeserializeAs<'de, u64> for NumAsHex {
    fn deserialize_as<D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NumAsHexVisitorU64)
    }
}

impl SerializeAs<u128> for NumAsHex {
    fn serialize_as<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{value:#x}"))
    }
}

impl<'de> DeserializeAs<'de, u128> for NumAsHex {
    fn deserialize_as<D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NumAsHexVisitorU128)
    }
}

impl Visitor<'_> for NumAsHexVisitorU64 {
    type Value = u64;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "string or number")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match u64::from_str_radix(v.trim_start_matches("0x"), 16) {
            Ok(value) => Ok(value),
            Err(err) => Err(serde::de::Error::custom(format!("invalid hex string: {err}"))),
        }
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match v.try_into() {
            Ok(value) => self.visit_u64(value),
            Err(_) => Err(serde::de::Error::custom(format!("value cannot be negative: {}", v))),
        }
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v)
    }
}

impl Visitor<'_> for NumAsHexVisitorU128 {
    type Value = u128;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "string or number")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match u128::from_str_radix(v.trim_start_matches("0x"), 16) {
            Ok(value) => Ok(value),
            Err(err) => Err(serde::de::Error::custom(format!("invalid hex string: {err}"))),
        }
    }
}
