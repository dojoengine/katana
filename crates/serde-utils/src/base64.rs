use std::fmt::{self, Formatter};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::de::{self, Visitor};
use serde::{Deserializer, Serializer};

/// Serializes an array of bytes as base64 string.
pub fn serialize<S, T>(value: T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: AsRef<[u8]>,
{
    serializer.serialize_str(&STANDARD.encode(value.as_ref()))
}

/// Deserializes [`Vec<u8>`] from base64 string.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    struct Base64Visitor;

    impl Visitor<'_> for Base64Visitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
            write!(formatter, "base64 encoded string")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            STANDARD.decode(v).map_err(|e| de::Error::custom(format!("invalid base64 string: {e}")))
        }
    }

    deserializer.deserialize_any(Base64Visitor)
}
