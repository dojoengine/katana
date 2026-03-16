use std::fmt::Debug;

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

const ENVELOPE_FORMAT_VERSION: u8 = 1;
const ENVELOPE_HEADER_LEN: usize = 4 + 1 + 1 + 2; // magic (4) + version (1) + encoding (1) + flags (2)

/// Per-record feature flags. Stored as 2 bytes little-endian in the header.
///
/// All 16 bits are reserved for future use (checksum, encryption, etc).
const ENVELOPE_FLAGS_RESERVED: u16 = 0x0000;

/// Minimum zstd frame size in bytes (magic 4 + frame header 2 + block header 3 + checksum 4).
///
/// The actual frame header is variable-length (2–14 bytes) depending on window size, dictionary ID,
/// and content size fields; this constant uses the absolute minimum.
///
/// Payloads smaller than this are stored uncompressed because the zstd frame overhead would exceed
/// any savings.
const ZSTD_MIN_FRAME_SIZE: usize = 13;

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
    #[error("unsupported {name} envelope encoding: {source}")]
    UnsupportedEncoding {
        name: &'static str,
        #[source]
        source: UnknownEncodingError,
    },

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
pub trait EnvelopePayload: Compress + Decompress + Debug + Clone + PartialEq + Eq {
    /// 4-byte magic identifier for this payload type.
    const MAGIC: &'static [u8; 4];
    /// Human-readable name for error messages.
    const NAME: &str;
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
/// | MAGIC   | VERSION | ENCODING | FLAGS   | PAYLOAD                     |
/// +---------+---------+----------+---------+-----------------------------+
/// | 4 bytes | 1 byte  | 1 byte   | 2 bytes | variable length             |
/// +---------+---------+----------+---------+-----------------------------+
/// ```
///
/// **Magic** (bytes 0–3):
///
/// A 4-byte ASCII tag unique to each payload type (e.g. `KRCP` for receipts, `KTXN` for
/// transactions). Used to distinguish envelope-encoded rows from legacy rows during decompression.
///
/// **Format version** (byte 4):
///
/// Schema version of the envelope layout itself. Currently `0x01`. A reader that encounters a
/// higher version will reject the row, ensuring forward-incompatible changes are caught early.
///
/// **Encoding** (byte 5):
///
/// Identifies the compression algorithm applied to the serialized payload.
///   - `0x00` — identity (no compression).
///   - `0x01` — zstd (default level, no dictionary).
///
/// **Flags** (bytes 6–7):
///
/// Reserved 16-bit little-endian field for future per-record features (checksum, encryption, etc).
/// Currently always `0x0000`.
///
/// **Payload** (bytes 8..):
///
/// The inner value serialized by the payload's own `to_bytes` method, then optionally compressed
/// according to the encoding byte.
///
/// ## Backward Compatibility
///
/// Envelope will fallback to decode the payload as the inner type directly if the magic bytes
/// do not match the expected value. This allows newer clients to read older pre-envelope data
/// without requiring a database migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope<T: EnvelopePayload> {
    pub inner: T,
}

impl<T: EnvelopePayload> Envelope<T> {
    fn do_compress(self) -> Result<Vec<u8>, EnvelopeError> {
        let serialized = self.inner.compress().unwrap();
        let serialized_len = serialized.as_ref().len();

        let (encoding, payload) = if serialized_len < ZSTD_MIN_FRAME_SIZE {
            (Encoding::Identity, serialized.into())
        } else {
            let compressed = zstd::encode_all(serialized.as_ref(), 0).map_err(|e| {
                EnvelopeError::ZstdCompress { name: T::NAME, reason: e.to_string() }
            })?;

            (Encoding::Zstd, compressed)
        };

        let mut encoded = Vec::with_capacity(ENVELOPE_HEADER_LEN + payload.len());
        encoded.extend_from_slice(T::MAGIC);
        encoded.push(ENVELOPE_FORMAT_VERSION);
        encoded.push(encoding as u8);
        encoded.extend_from_slice(&ENVELOPE_FLAGS_RESERVED.to_le_bytes());
        encoded.extend_from_slice(&payload);

        Ok(encoded)
    }

    fn do_decompress(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        if !bytes.starts_with(T::MAGIC) {
            return T::decompress(bytes)
                .map(|inner| Self { inner })
                .map_err(|e| EnvelopeError::LegacyDecode { name: T::NAME, reason: e.to_string() });
        }

        if bytes.len() < ENVELOPE_HEADER_LEN {
            return Err(EnvelopeError::IncompleteHeader {
                name: T::NAME,
                actual: bytes.len(),
                expected: ENVELOPE_HEADER_LEN,
            });
        }

        let version = bytes[4];
        if version != ENVELOPE_FORMAT_VERSION {
            return Err(EnvelopeError::UnsupportedVersion { name: T::NAME, version });
        }

        let encoding = bytes[5];
        let encoding = Encoding::try_from(encoding)
            .map_err(|source| EnvelopeError::UnsupportedEncoding { name: T::NAME, source })?;

        // bytes 6–7: flags (reserved, read but unused for now)
        let _flags = u16::from_le_bytes([bytes[6], bytes[7]]);

        let decoded = match encoding {
            Encoding::Identity => bytes[ENVELOPE_HEADER_LEN..].to_vec(),
            Encoding::Zstd => zstd::decode_all(&bytes[ENVELOPE_HEADER_LEN..]).map_err(|e| {
                EnvelopeError::ZstdDecompress { name: T::NAME, reason: e.to_string() }
            })?,
        };

        Ok(Self { inner: T::decompress(&decoded).unwrap() })
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

impl<T: EnvelopePayload> From<T> for Envelope<T> {
    fn from(inner: T) -> Self {
        Self { inner }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown encoding format `{0}`")]
pub struct UnknownEncodingError(pub u8);

/// Compression encoding applied to the serialized payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Encoding {
    /// No compression. Payload stored as raw serialized bytes.
    Identity = 0x00,
    /// Zstd compression (default level, no dictionary).
    Zstd = 0x01,
}

impl TryFrom<u8> for Encoding {
    type Error = UnknownEncodingError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Encoding::Identity),
            0x01 => Ok(Encoding::Zstd),
            _ => Err(UnknownEncodingError(value)),
        }
    }
}

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
    }

    impl Compress for TestPayload {
        type Compressed = Vec<u8>;
        fn compress(self) -> Result<Self::Compressed, CodecError> {
            Ok(self.data.as_bytes().to_vec())
        }
    }

    impl Decompress for TestPayload {
        fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
            Ok(Self { data: String::from_utf8(bytes.as_ref().to_vec()).unwrap() })
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
        assert_eq!(compressed[4], ENVELOPE_FORMAT_VERSION);
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
        assert!(payload.data.len() < ZSTD_MIN_FRAME_SIZE);
        let envelope = Envelope::from(payload.clone());

        let compressed = envelope.compress().expect("compress");
        assert_eq!(&compressed[..4], b"TEST");
        assert_eq!(compressed[4], ENVELOPE_FORMAT_VERSION);
        assert_eq!(compressed[5], Encoding::Identity as u8);
        assert_eq!(&compressed[6..8], &[0x00, 0x00]);
        // payload follows directly after the 8-byte header
        assert_eq!(&compressed[ENVELOPE_HEADER_LEN..], payload.data.as_bytes());

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
        encoded.push(ENVELOPE_FORMAT_VERSION + 1);
        encoded.push(Encoding::Zstd as u8);
        encoded.extend_from_slice(&ENVELOPE_FLAGS_RESERVED.to_le_bytes());

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::UnsupportedVersion { version: 2, .. }));
    }

    #[test]
    fn envelope_rejects_unknown_encoding() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(ENVELOPE_FORMAT_VERSION);
        encoded.push(0xFF); // bad encoding
        encoded.extend_from_slice(&ENVELOPE_FLAGS_RESERVED.to_le_bytes());

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::UnsupportedEncoding { .. }));
    }

    #[test]
    fn envelope_rejects_corrupt_payload() {
        let mut encoded = b"TEST".to_vec();
        encoded.push(ENVELOPE_FORMAT_VERSION);
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
        encoded.push(ENVELOPE_FORMAT_VERSION);

        let err = Envelope::<TestPayload>::do_decompress(&encoded).expect_err("must reject");
        assert!(matches!(err, EnvelopeError::IncompleteHeader { expected: 8, actual: 5, .. }));
    }
}
