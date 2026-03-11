use std::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

const FORMAT_VERSION: u8 = 1;
const ENCODING_ZSTD: u8 = 1;
const HEADER_LEN: usize = 4 + 2; // magic (4) + version (1) + encoding (1)

/// Concrete error type for envelope compress/decompress operations.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    /// The envelope header is present but truncated (fewer than [`HEADER_LEN`] bytes).
    #[error("incomplete {name} envelope: expected at least {expected} bytes, got {actual}")]
    IncompleteHeader { name: &'static str, expected: usize, actual: usize },

    /// The format version byte is not recognised by this build.
    #[error("unsupported {name} envelope version: {version}")]
    UnsupportedVersion { name: &'static str, version: u8 },

    /// The encoding byte is not recognised by this build.
    #[error("unsupported {name} envelope encoding: {encoding}")]
    UnsupportedEncoding { name: &'static str, encoding: u8 },

    /// `postcard` serialization failed during compression.
    #[error("failed to serialize {name} payload: {reason}")]
    Serialize { name: &'static str, reason: String },

    /// `postcard` deserialization failed after decompression.
    #[error("failed to deserialize {name} payload: {reason}")]
    Deserialize { name: &'static str, reason: String },

    /// `zstd` compression failed.
    #[error("failed to zstd-compress {name} payload: {reason}")]
    ZstdCompress { name: &'static str, reason: String },

    /// `zstd` decompression failed (corrupt or truncated data).
    #[error("failed to zstd-decompress {name} payload: {reason}")]
    ZstdDecompress { name: &'static str, reason: String },

    /// Legacy (pre-envelope) deserialization failed.
    #[error("failed to decode legacy {name} bytes: {reason}")]
    LegacyDecode { name: &'static str, reason: String },
}

impl From<EnvelopeError> for CodecError {
    fn from(err: EnvelopeError) -> Self {
        match &err {
            EnvelopeError::Serialize { .. } | EnvelopeError::ZstdCompress { .. } => {
                CodecError::Compress(err.to_string())
            }
            _ => CodecError::Decompress(err.to_string()),
        }
    }
}

/// Trait for types that can be stored in a compressed envelope.
pub trait EnvelopePayload: Serialize + DeserializeOwned + Debug + Clone + PartialEq + Eq {
    /// 4-byte magic identifier for this payload type.
    const MAGIC: &[u8; 4];
    /// Human-readable name for error messages.
    const NAME: &str;

    /// Try to deserialize from legacy (pre-envelope) bytes.
    fn from_legacy_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError>;
}

/// Generic compressed envelope for on-disk table values.
///
/// Wraps a payload `T` with a fixed 6-byte header so that encoding can evolve
/// independently from the in-memory types. Rows written before the envelope
/// existed (legacy rows) are detected by the absence of the magic prefix and
/// decoded via [`EnvelopePayload::from_legacy_bytes`].
///
/// # Wire format
///
/// ```text
/// +---------+-----------+------------+-------------------------------+
/// | magic   | version   | encoding   | payload                       |
/// | 4 bytes | 1 byte    | 1 byte     | variable length               |
/// +---------+-----------+------------+-------------------------------+
/// ```
///
/// **Magic** (bytes 0–3): A 4-byte ASCII tag unique to each payload type
/// (e.g. `KRCP` for receipts, `KTXN` for transactions). Used to distinguish
/// envelope-encoded rows from legacy rows during decompression.
///
/// **Format version** (byte 4): Schema version of the envelope layout itself.
/// Currently `0x01`. A reader that encounters a higher version will reject the
/// row, ensuring forward-incompatible changes are caught early.
///
/// **Encoding** (byte 5): Identifies the compression algorithm applied to the
/// serialized payload.
///   - `0x01` — zstd (default level, no dictionary).
///   - `0x02` — reserved for zstd with a shared dictionary.
///
/// **Payload** (bytes 6..): The inner value serialized with
/// [postcard](https://docs.rs/postcard) and then compressed according to the
/// encoding byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope<T: EnvelopePayload> {
    pub inner: T,
}

impl<T: EnvelopePayload> From<T> for Envelope<T> {
    fn from(inner: T) -> Self {
        Self { inner }
    }
}

impl<T: EnvelopePayload> Envelope<T> {
    pub(crate) fn do_compress(self) -> Result<Vec<u8>, EnvelopeError> {
        let serialized = postcard::to_stdvec(&self.inner)
            .map_err(|e| EnvelopeError::Serialize { name: T::NAME, reason: e.to_string() })?;

        let compressed = zstd::encode_all(serialized.as_slice(), 0)
            .map_err(|e| EnvelopeError::ZstdCompress { name: T::NAME, reason: e.to_string() })?;

        let mut encoded = Vec::with_capacity(HEADER_LEN + compressed.len());
        encoded.extend_from_slice(T::MAGIC);
        encoded.push(FORMAT_VERSION);
        encoded.push(ENCODING_ZSTD);
        encoded.extend_from_slice(&compressed);

        Ok(encoded)
    }

    pub(crate) fn do_decompress(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        if bytes.starts_with(T::MAGIC) {
            if bytes.len() < HEADER_LEN {
                return Err(EnvelopeError::IncompleteHeader {
                    name: T::NAME,
                    expected: HEADER_LEN,
                    actual: bytes.len(),
                });
            }

            let version = bytes[T::MAGIC.len()];
            if version != FORMAT_VERSION {
                return Err(EnvelopeError::UnsupportedVersion { name: T::NAME, version });
            }

            let encoding = bytes[T::MAGIC.len() + 1];
            if encoding != ENCODING_ZSTD {
                return Err(EnvelopeError::UnsupportedEncoding { name: T::NAME, encoding });
            }

            let serialized = zstd::decode_all(&bytes[HEADER_LEN..]).map_err(|e| {
                EnvelopeError::ZstdDecompress { name: T::NAME, reason: e.to_string() }
            })?;

            let inner = postcard::from_bytes(&serialized)
                .map_err(|e| EnvelopeError::Deserialize { name: T::NAME, reason: e.to_string() })?;

            Ok(Self { inner })
        } else {
            T::from_legacy_bytes(bytes).map(|inner| Self { inner })
        }
    }
}

impl<T: EnvelopePayload> Compress for Envelope<T> {
    type Compressed = Vec<u8>;

    fn compress(self) -> Result<Self::Compressed, CodecError> {
        self.do_compress().map_err(Into::into)
    }
}

impl<T: EnvelopePayload> Decompress for Envelope<T> {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        Self::do_decompress(bytes.as_ref()).map_err(Into::into)
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

        fn from_legacy_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
            postcard::from_bytes(bytes).map_err(|e| EnvelopeError::LegacyDecode {
                name: Self::NAME,
                reason: e.to_string(),
            })
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

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::UnsupportedVersion { version: 2, .. }));
    }

    #[test]
    fn envelope_rejects_unknown_encoding() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION);
        encoded.push(ENCODING_ZSTD + 1);

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::UnsupportedEncoding { encoding: 2, .. }));
    }

    #[test]
    fn envelope_rejects_corrupt_payload() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION);
        encoded.push(ENCODING_ZSTD);
        encoded.extend_from_slice(&[1, 2, 3, 4]);

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::ZstdDecompress { .. }));
    }

    #[test]
    fn envelope_rejects_incomplete_header() {
        let encoded = b"TEST".to_vec(); // magic only, missing version + encoding

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::IncompleteHeader { expected: 6, actual: 4, .. }));
    }
}
