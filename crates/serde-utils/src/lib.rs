use serde::de::Visitor;
use serde::{Deserialize, Deserializer};

pub mod base64;

/// Serializes a value as a hexadecimal string with "0x" prefix.
pub fn serialize_as_hex<S, T>(value: &T, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: serde::Serialize + std::fmt::LowerHex,
{
    serializer.serialize_str(&format!("{value:#x}"))
}

/// Serializes an optional value as a hexadecimal string with "0x" prefix, or as null if None.
pub fn serialize_opt_as_hex<S, T>(
    value: &Option<T>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: serde::Serialize + std::fmt::LowerHex,
{
    match value {
        Some(value) => serializer.serialize_str(&format!("{value:#x}")),
        None => serializer.serialize_none(),
    }
}

/// Deserializes a `u64` from either a hexadecimal string with "0x" prefix or a decimal
/// string/number.
pub fn deserialize_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    struct U64HexVisitor;

    impl Visitor<'_> for U64HexVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "0x-prefix hex string or decimal number")
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
            if let Some(hex) = v.strip_prefix("0x") {
                u64::from_str_radix(hex, 16).map_err(serde::de::Error::custom)
            } else {
                v.parse::<u64>().map_err(serde::de::Error::custom)
            }
        }
    }

    deserializer.deserialize_any(U64HexVisitor)
}

/// Deserializes a `u128` from either a hexadecimal string with "0x" prefix or a decimal
/// string/number.
pub fn deserialize_u128<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
    struct U128HexVisitor;

    impl Visitor<'_> for U128HexVisitor {
        type Value = u128;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "0x-prefix hex string or decimal number")
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
            if let Some(hex) = v.strip_prefix("0x") {
                u128::from_str_radix(hex, 16).map_err(serde::de::Error::custom)
            } else {
                v.parse::<u128>().map_err(serde::de::Error::custom)
            }
        }
    }

    deserializer.deserialize_any(U128HexVisitor)
}

/// Deserializes an optional `u64` from either a hexadecimal string with "0x" prefix, a decimal
/// string/number, or null.
pub fn deserialize_opt_u64<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNum {
        String(String),
        Number(u64),
    }

    match Option::<StringOrNum>::deserialize(deserializer)? {
        None => Ok(None),
        Some(StringOrNum::Number(n)) => Ok(Some(n)),
        Some(StringOrNum::String(s)) => {
            if let Some(hex) = s.strip_prefix("0x") {
                u64::from_str_radix(hex, 16).map(Some).map_err(serde::de::Error::custom)
            } else {
                s.parse().map(Some).map_err(serde::de::Error::custom)
            }
        }
    }
}

/// Deserializes an optional `u128` from either a hexadecimal string with "0x" prefix, a decimal
/// string/number, or null.
pub fn deserialize_opt_u128<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u128>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNum {
        String(String),
        Number(u128),
    }

    match Option::<StringOrNum>::deserialize(deserializer)? {
        None => Ok(None),
        Some(StringOrNum::Number(n)) => Ok(Some(n)),
        Some(StringOrNum::String(s)) => {
            if let Some(hex) = s.strip_prefix("0x") {
                u128::from_str_radix(hex, 16).map(Some).map_err(serde::de::Error::custom)
            } else {
                s.parse().map(Some).map_err(serde::de::Error::custom)
            }
        }
    }
}
