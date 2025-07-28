use serde::Deserialize;

pub fn serialize_hex_u64<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: ::serde::Serializer,
{
    serializer.serialize_str(&format!("{value:#x}"))
}

pub fn serialize_hex_u128<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
where
    S: ::serde::Serializer,
{
    serializer.serialize_str(&format!("{value:#x}"))
}

pub fn deserialize_hex_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: ::serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16)
        .map_err(|e| ::serde::de::Error::custom(format!("invalid hex string: {e}")))
}

pub fn deserialize_hex_u128<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: ::serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    u128::from_str_radix(s.trim_start_matches("0x"), 16)
        .map_err(|e| ::serde::de::Error::custom(format!("invalid hex string: {e}")))
}
