use std::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

const FORMAT_VERSION: u8 = 1;
const ENCODING_ZSTD: u8 = 1;
const HEADER_LEN: usize = 4 + 2; // magic (4) + version (1) + encoding (1)

/// Trait for types that can be stored in a compressed envelope.
pub trait EnvelopePayload: Serialize + DeserializeOwned + Debug + Clone + PartialEq + Eq {
    /// 4-byte magic identifier for this payload type.
    const MAGIC: &[u8; 4];
    /// Human-readable name for error messages.
    const NAME: &str;

    /// Try to deserialize from legacy (pre-envelope) bytes.
    fn from_legacy_bytes(bytes: &[u8]) -> Result<Self, CodecError>;
}

/// Generic compressed envelope.
///
/// ```text
/// +---------+-----------+------------+-------------------------------+
/// | magic   | version   | encoding   | payload                       |
/// | 4 bytes | 1 byte    | 1 byte     | variable length               |
/// +---------+-----------+------------+-------------------------------+
/// | MAGIC   | 0x01      | 0x01       | zstd(postcard(T))             |
/// +---------+-----------+------------+-------------------------------+
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope<T: EnvelopePayload> {
    pub inner: T,
}

impl<T: EnvelopePayload> From<T> for Envelope<T> {
    fn from(inner: T) -> Self {
        Self { inner }
    }
}

impl<T: EnvelopePayload> Compress for Envelope<T> {
    type Compressed = Vec<u8>;

    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let serialized =
            postcard::to_stdvec(&self.inner).map_err(|e| CodecError::Compress(e.to_string()))?;
        let compressed = zstd::encode_all(serialized.as_slice(), 0)
            .map_err(|e| CodecError::Compress(e.to_string()))?;

        let mut encoded = Vec::with_capacity(HEADER_LEN + compressed.len());
        encoded.extend_from_slice(T::MAGIC);
        encoded.push(FORMAT_VERSION);
        encoded.push(ENCODING_ZSTD);
        encoded.extend_from_slice(&compressed);

        Ok(encoded)
    }
}

impl<T: EnvelopePayload> Decompress for Envelope<T> {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();

        if bytes.starts_with(T::MAGIC) {
            if bytes.len() < HEADER_LEN {
                return Err(CodecError::Decompress(format!("Incomplete {} envelope", T::NAME)));
            }

            let version = bytes[T::MAGIC.len()];
            if version != FORMAT_VERSION {
                return Err(CodecError::Decompress(format!(
                    "Unsupported {} encoding version: {version}",
                    T::NAME
                )));
            }

            let encoding = bytes[T::MAGIC.len() + 1];
            if encoding != ENCODING_ZSTD {
                return Err(CodecError::Decompress(format!(
                    "Unsupported {} encoding: {encoding}",
                    T::NAME
                )));
            }

            let serialized = zstd::decode_all(&bytes[HEADER_LEN..])
                .map_err(|e| CodecError::Decompress(e.to_string()))?;
            let inner = postcard::from_bytes(&serialized)
                .map_err(|e| CodecError::Decompress(e.to_string()))?;
            Ok(Self { inner })
        } else {
            T::from_legacy_bytes(bytes).map(|inner| Self { inner })
        }
    }
}

pub type TxEnvelope = Envelope<crate::models::versioned::transaction::VersionedTx>;

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct TestPayload {
        data: String,
    }

    impl EnvelopePayload for TestPayload {
        const MAGIC: &[u8; 4] = b"TEST";
        const NAME: &str = "test";

        fn from_legacy_bytes(bytes: &[u8]) -> Result<Self, CodecError> {
            postcard::from_bytes(bytes).map_err(|e| CodecError::Decompress(e.to_string()))
        }
    }

    fn sample_payload() -> TestPayload {
        TestPayload { data: "hello world".into() }
    }

    #[test]
    fn envelope_roundtrip() {
        let payload = sample_payload();
        let envelope = Envelope::from(payload.clone());

        let compressed = envelope.compress().expect("compress");
        assert_eq!(&compressed[..4], b"TEST");
        assert_eq!(compressed[4], FORMAT_VERSION);
        assert_eq!(compressed[5], ENCODING_ZSTD);

        let decompressed = Envelope::<TestPayload>::decompress(compressed).expect("decompress");
        assert_eq!(decompressed.inner, payload);
    }

    #[test]
    fn envelope_decompresses_legacy_bytes() {
        let payload = sample_payload();
        let legacy = postcard::to_stdvec(&payload).expect("serialize");

        let decompressed = Envelope::<TestPayload>::decompress(legacy).expect("decompress");
        assert_eq!(decompressed.inner, payload);
    }

    #[test]
    fn envelope_rejects_unknown_version() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION + 1);
        encoded.push(ENCODING_ZSTD);

        let err = Envelope::<TestPayload>::decompress(encoded).expect_err("must reject");
        assert!(matches!(err, CodecError::Decompress(_)));
    }

    #[test]
    fn envelope_rejects_unknown_encoding() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION);
        encoded.push(ENCODING_ZSTD + 1);

        let err = Envelope::<TestPayload>::decompress(encoded).expect_err("must reject");
        assert!(matches!(err, CodecError::Decompress(_)));
    }

    #[test]
    fn envelope_rejects_corrupt_payload() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION);
        encoded.push(ENCODING_ZSTD);
        encoded.extend_from_slice(&[1, 2, 3, 4]);

        let err = Envelope::<TestPayload>::decompress(encoded).expect_err("must reject");
        assert!(matches!(err, CodecError::Decompress(_)));
    }
}
