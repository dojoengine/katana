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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AllResourceBoundsMapping {
    /// L1 gas bounds - covers L2→L1 messages sent by the transaction
    pub l1_gas: ResourceBounds,
    /// L2 gas bounds - covers L2 resources including computation, tx payload, event emission, code
    /// size, etc. Units: 1 Cairo step = 100 L2 gas
    pub l2_gas: ResourceBounds,
    /// L1 data gas (blob gas) bounds - covers the cost of submitting state diffs as blobs on L1
    pub l1_data_gas: ResourceBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct L1GasResourceBoundsMapping {
    /// L1 gas bounds - covers L2→L1 messages sent by the transaction
    pub l1_gas: ResourceBounds,

    /// L2 gas bounds - covers L2 resources including computation, tx payload, event emission, code
    /// size, etc. Units: 1 Cairo step = 100 L2 gas.
    ///
    /// Pre 0.13.3. this field is signed but never used.
    pub l2_gas: ResourceBounds,
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
pub enum ResourceBoundsMapping {
    /// Legacy bounds; only L1 gas bounds specified (backward compatibility).
    ///
    /// Raw resources are converted to L1 gas for cost calculation. Prior to 0.14.0, the L2 gas
    /// bounds is signed but is always hardcoded to be zero thus, the L2 gas field is completely
    /// ommitted from this variant and is assumed to be zero during transaction hash computation.
    ///
    /// Supported in Starknet v0.13.4 but rejected in v0.14.0+.
    L1Gas(L1GasResourceBoundsMapping),

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

#[cfg(feature = "serde")]
impl serde::Serialize for ResourceBounds {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        if serializer.is_human_readable() {
            let mut state = serializer.serialize_struct("ResourceBounds", 2)?;
            state.serialize_field("max_amount", &format!("{:#x}", self.max_amount))?;
            state.serialize_field(
                "max_price_per_unit",
                &format!("{:#x}", self.max_price_per_unit),
            )?;
            state.end()
        } else {
            let mut state = serializer.serialize_struct("ResourceBounds", 2)?;
            state.serialize_field("max_amount", &self.max_amount)?;
            state.serialize_field("max_price_per_unit", &self.max_price_per_unit)?;
            state.end()
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ResourceBounds {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use std::fmt;

        use serde::de::{self, MapAccess, Visitor};
        use serde_json::Value;

        if deserializer.is_human_readable() {
            struct __Visitor;

            impl<'de> Visitor<'de> for __Visitor {
                type Value = ResourceBounds;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("struct ResourceBounds")
                }

                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                    let mut max_amount = None;
                    let mut max_price_per_unit = None;

                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "max_amount" => {
                                if max_amount.is_some() {
                                    return Err(de::Error::duplicate_field("max_amount"));
                                }
                                let value: Value = map.next_value()?;
                                max_amount = Some(match value {
                                    Value::String(s) => {
                                        if let Some(hex) = s.strip_prefix("0x") {
                                            u64::from_str_radix(hex, 16)
                                                .map_err(de::Error::custom)?
                                        } else {
                                            s.parse().map_err(de::Error::custom)?
                                        }
                                    }
                                    Value::Number(n) => n
                                        .as_u64()
                                        .ok_or_else(|| de::Error::custom("invalid u64"))?,
                                    _ => {
                                        return Err(de::Error::custom(
                                            "expected 0x-prefix hex string or number for \
                                             max_amount",
                                        ))
                                    }
                                });
                            }

                            "max_price_per_unit" => {
                                if max_price_per_unit.is_some() {
                                    return Err(de::Error::duplicate_field("max_price_per_unit"));
                                }

                                let value: Value = map.next_value()?;
                                max_price_per_unit = Some(match value {
                                    Value::String(s) => {
                                        if let Some(hex) = s.strip_prefix("0x") {
                                            u128::from_str_radix(hex, 16)
                                                .map_err(de::Error::custom)?
                                        } else {
                                            s.parse().map_err(de::Error::custom)?
                                        }
                                    }
                                    Value::Number(n) => {
                                        if let Some(u) = n.as_u64() {
                                            u as u128
                                        } else {
                                            return Err(de::Error::custom("invalid u128"));
                                        }
                                    }
                                    _ => {
                                        return Err(de::Error::custom(
                                            "expected 0x-prefix hex string or number for \
                                             max_price_per_unit",
                                        ))
                                    }
                                });
                            }

                            _ => {
                                let _ = map.next_value::<serde::de::IgnoredAny>()?;
                            }
                        }
                    }

                    let max_amount =
                        max_amount.ok_or_else(|| de::Error::missing_field("max_amount"))?;
                    let max_price_per_unit = max_price_per_unit
                        .ok_or_else(|| de::Error::missing_field("max_price_per_unit"))?;

                    Ok(ResourceBounds { max_amount, max_price_per_unit })
                }
            }

            deserializer.deserialize_struct(
                "ResourceBounds",
                &["max_amount", "max_price_per_unit"],
                __Visitor,
            )
        } else {
            #[derive(serde::Deserialize)]
            struct ResourceBoundsBinary {
                max_amount: u64,
                max_price_per_unit: u128,
            }

            let binary = ResourceBoundsBinary::deserialize(deserializer)?;
            Ok(ResourceBounds {
                max_amount: binary.max_amount,
                max_price_per_unit: binary.max_price_per_unit,
            })
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ResourceBoundsMapping {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        // For human readable formats (primarily targetting JSON), serialize as a flat object.
        if serializer.is_human_readable() {
            match self {
                ResourceBoundsMapping::L1Gas(bounds) => {
                    let mut state = serializer.serialize_struct("ResourceBoundsMapping", 2)?;
                    state.serialize_field("l2_gas", &bounds.l2_gas)?;
                    state.serialize_field("l1_gas", &bounds.l1_gas)?;
                    state.end()
                }
                ResourceBoundsMapping::All(bounds) => {
                    let mut state = serializer.serialize_struct("ResourceBoundsMapping", 3)?;
                    state.serialize_field("l2_gas", &bounds.l2_gas)?;
                    state.serialize_field("l1_gas", &bounds.l1_gas)?;
                    state.serialize_field("l1_data_gas", &bounds.l1_data_gas)?;
                    state.end()
                }
            }
        }
        // For binary formats, use explicit enum tagging:
        //
        // * ResourceBoundsMapping::L1Gas = 0
        // * ResourceBoundsMapping::All = 1
        else {
            match self {
                ResourceBoundsMapping::L1Gas(v) => {
                    serializer.serialize_newtype_variant("ResourceBoundsMapping", 0, "L1Gas", v)
                }
                ResourceBoundsMapping::All(v) => {
                    serializer.serialize_newtype_variant("ResourceBoundsMapping", 1, "All", v)
                }
            }
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ResourceBoundsMapping {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use std::fmt;

        use serde::de::{self, EnumAccess, VariantAccess, Visitor};

        if deserializer.is_human_readable() {
            // For JSON: deserialize from unified object format
            #[derive(serde::Deserialize)]
            struct UnifiedResourceBounds {
                l1_gas: ResourceBounds,
                l2_gas: ResourceBounds,
                l1_data_gas: Option<ResourceBounds>,
            }

            let unified = UnifiedResourceBounds::deserialize(deserializer)?;

            // If l1_data_gas is present, it's the All variant
            if let Some(l1_data_gas) = unified.l1_data_gas {
                Ok(ResourceBoundsMapping::All(AllResourceBoundsMapping {
                    l1_gas: unified.l1_gas,
                    l2_gas: unified.l2_gas,
                    l1_data_gas,
                }))
            } else {
                // Otherwise it's the L1Gas variant
                Ok(ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                    l1_gas: unified.l1_gas,
                    l2_gas: unified.l2_gas,
                }))
            }
        }
        // For binary formats, use standard enum deserialization (when derived using
        // #[derive(Deserialize)])
        else {
            struct __Visitor;

            impl<'de> Visitor<'de> for __Visitor {
                type Value = ResourceBoundsMapping;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("ResourceBoundsMapping enum")
                }

                fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
                    let (variant_idx, variant) = data.variant::<u32>()?;

                    match variant_idx {
                        0 => {
                            let value = variant.newtype_variant::<L1GasResourceBoundsMapping>()?;
                            Ok(ResourceBoundsMapping::L1Gas(value))
                        }
                        1 => {
                            let value = variant.newtype_variant::<AllResourceBoundsMapping>()?;
                            Ok(ResourceBoundsMapping::All(value))
                        }
                        _ => Err(de::Error::custom("invalid variant index; expected 0 or 1")),
                    }
                }
            }

            deserializer.deserialize_enum("ResourceBoundsMapping", &["L1Gas", "All"], __Visitor)
        }
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

    #[cfg(feature = "serde")]
    #[test]
    fn resource_bounds_mapping_json_serde() {
        use serde_json::json;

        // -------------------------------------------
        // Legacy resource bounds mapping

        let json = json!({
            "l2_gas": {
                "max_amount": "0x7d0",
                "max_price_per_unit": "0xc8"
            },
            "l1_gas": {
                "max_amount": "0x3e8",
                "max_price_per_unit": "0x64"
            }
        });

        let bounds = ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 1000, max_price_per_unit: 100 },
            l2_gas: ResourceBounds { max_amount: 2000, max_price_per_unit: 200 },
        });

        let serialized = serde_json::to_value(&bounds).unwrap();
        similar_asserts::assert_eq!(json, serialized);

        let deserialized: ResourceBoundsMapping = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, bounds);

        // -------------------------------------------
        // All resource bounds mapping

        let json = json!({
            "l2_gas": {
                "max_amount": "0x7d0",
                "max_price_per_unit": "0xc8"
            },
            "l1_gas": {
                "max_amount": "0x3e8",
                "max_price_per_unit": "0x64"
            },
            "l1_data_gas": {
                "max_amount": "0xbb8",
                "max_price_per_unit": "0x12c"
            }
        });

        let bounds = ResourceBoundsMapping::All(AllResourceBoundsMapping {
            l2_gas: ResourceBounds { max_amount: 2000, max_price_per_unit: 200 },
            l1_gas: ResourceBounds { max_amount: 1000, max_price_per_unit: 100 },
            l1_data_gas: ResourceBounds { max_amount: 3000, max_price_per_unit: 300 },
        });

        let serialized = serde_json::to_value(&bounds).unwrap();
        similar_asserts::assert_eq!(json, serialized);

        let deserialized: ResourceBoundsMapping = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, bounds);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn resource_bounds_mapping_binary_serde() {
        // Test L1Gas variant binary serialization (using postcard)
        let l1_gas_mapping = ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 1000, max_price_per_unit: 100 },
            l2_gas: ResourceBounds { max_amount: 2000, max_price_per_unit: 200 },
        });

        let binary = postcard::to_stdvec(&l1_gas_mapping).unwrap();
        let deserialized: ResourceBoundsMapping = postcard::from_bytes(&binary).unwrap();
        assert_eq!(deserialized, l1_gas_mapping);

        // Test All variant binary serialization
        let all_mapping = ResourceBoundsMapping::All(AllResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 1000, max_price_per_unit: 100 },
            l2_gas: ResourceBounds { max_amount: 2000, max_price_per_unit: 200 },
            l1_data_gas: ResourceBounds { max_amount: 3000, max_price_per_unit: 300 },
        });

        let binary = postcard::to_stdvec(&all_mapping).unwrap();
        let deserialized: ResourceBoundsMapping = postcard::from_bytes(&binary).unwrap();
        assert_eq!(deserialized, all_mapping);

        // Ensure binary format is different from JSON (uses enum tags)
        // Binary should be more compact than JSON
        let json_size = serde_json::to_string(&all_mapping).unwrap().len();
        assert!(binary.len() < json_size);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn resource_bounds_mapping_cross_format() {
        // Test that the same data structure can be serialized/deserialized
        // in both JSON and binary formats independently
        let mapping = ResourceBoundsMapping::All(AllResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 5000, max_price_per_unit: 500 },
            l2_gas: ResourceBounds { max_amount: 6000, max_price_per_unit: 600 },
            l1_data_gas: ResourceBounds { max_amount: 7000, max_price_per_unit: 700 },
        });

        // Serialize to JSON, deserialize, and verify
        let json = serde_json::to_string(&mapping).unwrap();
        let from_json: ResourceBoundsMapping = serde_json::from_str(&json).unwrap();
        assert_eq!(from_json, mapping);

        // Serialize to binary, deserialize, and verify
        let binary = postcard::to_stdvec(&mapping).unwrap();
        let from_binary: ResourceBoundsMapping = postcard::from_bytes(&binary).unwrap();
        assert_eq!(from_binary, mapping);

        // Verify that JSON and binary deserializations produce the same result
        assert_eq!(from_json, from_binary);
    }
}
