use std::fmt::Debug;

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

const FORMAT_VERSION: u8 = 1;
const HEADER_LEN: usize = 4 + 1 + 1 + 2; // magic (4) + version (1) + encoding (1) + flags (2)

/// Per-record feature flags. Stored as 2 bytes little-endian in the header.
/// All 16 bits are reserved for future use (checksum, encryption, etc).
const ENVELOPE_FLAGS_RESERVED: u16 = 0x0000;

/// Compression encoding applied to the serialized payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Encoding {
    /// No compression. Payload stored as raw serialized bytes.
    Identity = 0x00,
    /// Zstd compression (default level, no dictionary).
    Zstd = 0x01,
}

impl Encoding {
    fn from_u8(byte: u8, name: &'static str) -> Result<Self, EnvelopeError> {
        match byte {
            0x00 => Ok(Encoding::Identity),
            0x01 => Ok(Encoding::Zstd),
            _ => Err(EnvelopeError::UnsupportedEncoding { name, encoding: byte }),
        }
    }
}

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

    /// Payload serialization failed during compression.
    #[error("failed to encode {name} payload: {reason}")]
    Encode { name: &'static str, reason: String },

    /// Payload deserialization failed after decompression.
    #[error("failed to decode {name} payload: {reason}")]
    Decode { name: &'static str, reason: String },

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
            EnvelopeError::Encode { .. } | EnvelopeError::ZstdCompress { .. } => {
                CodecError::Compress(err.to_string())
            }
            _ => CodecError::Decompress(err.to_string()),
        }
    }
}

/// Trait for types that can be stored in a compressed envelope.
///
/// Unlike the previous version, payload types control their own serialization
/// format via [`to_bytes`](EnvelopePayload::to_bytes) and
/// [`from_bytes`](EnvelopePayload::from_bytes), so non-postcard formats
/// (JSON, protobuf, etc.) can be envelope-wrapped.
pub trait EnvelopePayload: Debug + Clone + PartialEq + Eq {
    /// 4-byte magic identifier for this payload type.
    const MAGIC: &[u8; 4];
    /// Human-readable name for error messages.
    const NAME: &str;

    /// Minimum serialized size (in bytes) below which compression is skipped.
    /// Set to `0` to always compress.
    const COMPRESS_THRESHOLD: usize = 0;

    /// Serialize this value to bytes. The format is payload-defined
    /// (postcard, JSON, protobuf, etc).
    fn to_bytes(&self) -> Result<Vec<u8>, EnvelopeError>;

    /// Deserialize from bytes produced by [`to_bytes`](EnvelopePayload::to_bytes).
    fn from_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError>;

    /// Try to deserialize from legacy (pre-envelope) bytes.
    fn from_legacy_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError>;
}

/// Generic compressed envelope for on-disk table values.
///
/// Wraps a payload `T` with a fixed 8-byte header so that encoding can evolve
/// independently from the in-memory types. Rows written before the envelope
/// existed (legacy rows) are detected by the absence of the magic prefix and
/// decoded via [`EnvelopePayload::from_legacy_bytes`].
///
/// # Wire format (v1)
///
/// ```text
/// +---------+---------+----------+---------+-----------------------------+
/// | magic   | version | encoding | flags   | payload                     |
/// | 4 bytes | 1 byte  | 1 byte   | 2 bytes | variable length             |
/// +---------+---------+----------+---------+-----------------------------+
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
///   - `0x00` — identity (no compression).
///   - `0x01` — zstd (default level, no dictionary).
///
/// **Flags** (bytes 6–7): Reserved 16-bit little-endian field for future
/// per-record features (checksum, encryption, etc). Currently always `0x0000`.
///
/// **Payload** (bytes 8..): The inner value serialized by the payload's own
/// `to_bytes` method, then optionally compressed according to the encoding byte.
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
        let serialized = self.inner.to_bytes()?;

        let (encoding, payload) =
            if T::COMPRESS_THRESHOLD > 0 && serialized.len() < T::COMPRESS_THRESHOLD {
                (Encoding::Identity, serialized)
            } else {
                let compressed = zstd::encode_all(serialized.as_slice(), 0).map_err(|e| {
                    EnvelopeError::ZstdCompress { name: T::NAME, reason: e.to_string() }
                })?;
                (Encoding::Zstd, compressed)
            };

        let flags = ENVELOPE_FLAGS_RESERVED;
        let mut encoded = Vec::with_capacity(HEADER_LEN + payload.len());
        encoded.extend_from_slice(T::MAGIC);
        encoded.push(FORMAT_VERSION);
        encoded.push(encoding as u8);
        encoded.extend_from_slice(&flags.to_le_bytes());
        encoded.extend_from_slice(&payload);

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

            let version = bytes[4];
            if version != FORMAT_VERSION {
                return Err(EnvelopeError::UnsupportedVersion { name: T::NAME, version });
            }

            let encoding = Encoding::from_u8(bytes[5], T::NAME)?;

            // bytes 6–7: flags (reserved, read but unused)
            let _flags = u16::from_le_bytes([bytes[6], bytes[7]]);

            let decoded = match encoding {
                Encoding::Identity => bytes[HEADER_LEN..].to_vec(),
                Encoding::Zstd => zstd::decode_all(&bytes[HEADER_LEN..]).map_err(|e| {
                    EnvelopeError::ZstdDecompress { name: T::NAME, reason: e.to_string() }
                })?,
            };

            let inner = T::from_bytes(&decoded)?;
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
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestPayload {
        data: String,
    }

    impl EnvelopePayload for TestPayload {
        const MAGIC: &[u8; 4] = b"TEST";
        const NAME: &str = "test";
        const COMPRESS_THRESHOLD: usize = 64;

        fn to_bytes(&self) -> Result<Vec<u8>, EnvelopeError> {
            Ok(self.data.as_bytes().to_vec())
        }

        fn from_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
            String::from_utf8(bytes.to_vec())
                .map(|data| Self { data })
                .map_err(|e| EnvelopeError::Decode { name: Self::NAME, reason: e.to_string() })
        }

        fn from_legacy_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
            Self::from_bytes(bytes)
        }
    }

    fn small_payload() -> TestPayload {
        TestPayload { data: "hello world".into() }
    }

    fn large_payload() -> TestPayload {
        TestPayload { data: "x".repeat(128) }
    }

    #[test]
    fn envelope_roundtrip() {
        let payload = large_payload();
        let envelope = Envelope::from(payload.clone());

        let compressed = envelope.compress().expect("compress");
        assert_eq!(&compressed[..4], b"TEST");
        assert_eq!(compressed[4], FORMAT_VERSION);
        assert_eq!(compressed[5], Encoding::Zstd as u8);
        assert_eq!(&compressed[6..8], &ENVELOPE_FLAGS_RESERVED.to_le_bytes());

        let decompressed = Envelope::<TestPayload>::decompress(compressed).expect("decompress");
        assert_eq!(decompressed.inner, payload);
    }

    #[test]
    fn envelope_decompresses_legacy_bytes() {
        let payload = small_payload();
        let legacy = payload.data.as_bytes().to_vec();

        let decompressed = Envelope::<TestPayload>::decompress(legacy).expect("decompress");
        assert_eq!(decompressed.inner, payload);
    }

    #[test]
    fn small_payload_uses_identity_encoding() {
        let payload = small_payload();
        let envelope = Envelope::from(payload.clone());

        let compressed = envelope.compress().expect("compress");
        assert_eq!(&compressed[..4], b"TEST");
        assert_eq!(compressed[4], FORMAT_VERSION);
        assert_eq!(compressed[5], Encoding::Identity as u8);
        assert_eq!(&compressed[6..8], &[0x00, 0x00]);
        // payload follows directly after the 8-byte header
        assert_eq!(&compressed[HEADER_LEN..], payload.data.as_bytes());

        let decompressed = Envelope::<TestPayload>::decompress(compressed).expect("decompress");
        assert_eq!(decompressed.inner, payload);
    }

    #[test]
    fn large_payload_uses_zstd_encoding() {
        let payload = large_payload();
        let envelope = Envelope::from(payload.clone());

        let compressed = envelope.compress().expect("compress");
        assert_eq!(compressed[5], Encoding::Zstd as u8);

        let decompressed = Envelope::<TestPayload>::decompress(compressed).expect("decompress");
        assert_eq!(decompressed.inner, payload);
    }

    #[test]
    fn envelope_rejects_unknown_version() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION + 1);
        encoded.push(Encoding::Zstd as u8);
        encoded.extend_from_slice(&ENVELOPE_FLAGS_RESERVED.to_le_bytes());

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::UnsupportedVersion { version: 2, .. }));
    }

    #[test]
    fn envelope_rejects_unknown_encoding() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION);
        encoded.push(0xFF); // bad encoding
        encoded.extend_from_slice(&ENVELOPE_FLAGS_RESERVED.to_le_bytes());

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::UnsupportedEncoding { encoding: 0xFF, .. }));
    }

    #[test]
    fn envelope_rejects_corrupt_payload() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION);
        encoded.push(Encoding::Zstd as u8);
        encoded.extend_from_slice(&ENVELOPE_FLAGS_RESERVED.to_le_bytes());
        encoded.extend_from_slice(&[1, 2, 3, 4]);

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::ZstdDecompress { .. }));
    }

    #[test]
    fn envelope_rejects_incomplete_header() {
        // magic + version only (5 bytes), missing encoding + flags
        let mut encoded = b"TEST".to_vec();
        encoded.push(FORMAT_VERSION);

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::IncompleteHeader { expected: 8, actual: 5, .. }));
    }
}
