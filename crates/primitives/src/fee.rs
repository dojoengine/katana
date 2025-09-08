#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// bounds is signed but is always hardcoded to be zero thus, the L2 gas field is completely
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
/// Transaction tip amount.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
pub struct Tip(u64);

impl Tip {
    /// Creates a new Tip with the given value
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

impl From<u64> for Tip {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<Tip> for u64 {
    fn from(tip: Tip) -> Self {
        tip.0
    }
}

impl std::fmt::Display for Tip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

impl std::str::FromStr for Tip {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(stripped) = s.strip_prefix("0x") {
            Ok(Self(u64::from_str_radix(stripped, 16)?))
        } else {
            Ok(Self(s.parse()?))
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Tip {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{:#x}", self.0))
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Tip {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use std::str::FromStr;

        let s = String::deserialize(deserializer)?;
        let tip = Tip::from_str(&s).map_err(serde::de::Error::custom)?;

        Ok(tip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "serde")]
    #[test]
    fn tip_serde() {
        // Test serialization
        let tip = Tip::new(0);
        let serialized = serde_json::to_string(&tip).unwrap();
        assert_eq!(serialized, "\"0x0\"");

        let tip = Tip::new(0xff);
        let serialized = serde_json::to_string(&tip).unwrap();
        assert_eq!(serialized, "\"0xff\"");

        let tip = Tip::new(0xa);
        let serialized = serde_json::to_string(&tip).unwrap();
        assert_eq!(serialized, "\"0xa\"");

        // Test deserialization with 0x prefix
        let deserialized: Tip = serde_json::from_str("\"0x0\"").unwrap();
        assert_eq!(deserialized.0, 0x0);

        let deserialized: Tip = serde_json::from_str("\"0xff\"").unwrap();
        assert_eq!(deserialized.0, 0xff);

        let deserialized: Tip = serde_json::from_str("\"0xa\"").unwrap();
        assert_eq!(deserialized.0, 0xa);

        // Test deserialization without 0x prefix (should fail)
        let result = serde_json::from_str::<Tip>("\"ff\"");
        assert!(result.is_err());

        let result = serde_json::from_str::<Tip>("\"a\"");
        assert!(result.is_err());

        // Test round-trip
        let original = Tip::new(12345);
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: Tip = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }
}
