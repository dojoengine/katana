//! A reference to data stored in a static file, or inline in MDBX.
//!
//! Used as the MDBX table value for tables whose heavy data has been moved to
//! static files. The MDBX entry serves as the authoritative gate — if the entry
//! exists in the MDBX transaction snapshot, the referenced static file data is
//! guaranteed to be durable.
//!
//! Wire format:
//! - Tag byte `0`: static file pointer — `[0] [offset: u64 LE] [length: u32 LE]` = 13 bytes
//! - Tag byte `1`: inline data — `[1] [compressed bytes...]`

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

const TAG_STATIC_FILE: u8 = 0;
const TAG_INLINE: u8 = 1;

/// A reference to data stored in a static file or inline in MDBX.
///
/// Used as the MDBX table value for tables whose heavy data lives in static files.
/// In sequential (production) mode, stores a pointer to a byte range in a `.dat` file.
/// In fork mode (non-sequential block numbers), stores the compressed data inline.
///
/// The MDBX entry is the authoritative gate: if it exists in the transaction snapshot,
/// the referenced data is guaranteed to be durable on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticFileRef {
    /// Data is in a static file at the given byte offset and length.
    StaticFile { offset: u64, length: u32 },
    /// Data is stored inline (e.g., for fork mode where static files aren't used).
    Inline(Vec<u8>),
}

impl StaticFileRef {
    /// Create a static file pointer.
    pub fn pointer(offset: u64, length: u32) -> Self {
        Self::StaticFile { offset, length }
    }

    /// Create an inline reference from already-compressed bytes.
    pub fn inline(compressed_bytes: Vec<u8>) -> Self {
        Self::Inline(compressed_bytes)
    }

    /// For a `StaticFile` pointer, returns `offset + length` (the byte position
    /// just past the end of this entry in the .dat file). Returns `None` for inline refs.
    pub fn byte_end(&self) -> Option<u64> {
        match self {
            StaticFileRef::StaticFile { offset, length } => Some(*offset + *length as u64),
            StaticFileRef::Inline(_) => None,
        }
    }
}

impl Compress for StaticFileRef {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        match self {
            StaticFileRef::StaticFile { offset, length } => {
                let mut buf = Vec::with_capacity(13);
                buf.push(TAG_STATIC_FILE);
                buf.extend_from_slice(&offset.to_le_bytes());
                buf.extend_from_slice(&length.to_le_bytes());
                Ok(buf)
            }
            StaticFileRef::Inline(data) => {
                let mut buf = Vec::with_capacity(1 + data.len());
                buf.push(TAG_INLINE);
                buf.extend_from_slice(&data);
                Ok(buf)
            }
        }
    }
}

impl Decompress for StaticFileRef {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();
        if bytes.is_empty() {
            return Err(CodecError::Decode("empty StaticFileRef".into()));
        }
        match bytes[0] {
            TAG_STATIC_FILE => {
                if bytes.len() != 13 {
                    return Err(CodecError::Decode(format!(
                        "StaticFileRef pointer expected 13 bytes, got {}",
                        bytes.len()
                    )));
                }
                let offset = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
                let length = u32::from_le_bytes(bytes[9..13].try_into().unwrap());
                Ok(StaticFileRef::StaticFile { offset, length })
            }
            TAG_INLINE => {
                let data = bytes[1..].to_vec();
                Ok(StaticFileRef::Inline(data))
            }
            tag => Err(CodecError::Decode(format!("unknown StaticFileRef tag: {tag}"))),
        }
    }
}

impl std::fmt::Display for StaticFileRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StaticFileRef::StaticFile { offset, length } => {
                write!(f, "StaticFile(offset={offset}, length={length})")
            }
            StaticFileRef::Inline(data) => write!(f, "Inline({} bytes)", data.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_roundtrip() {
        let ptr = StaticFileRef::pointer(12345, 678);
        let compressed = ptr.clone().compress().unwrap();
        assert_eq!(compressed.len(), 13);
        assert_eq!(compressed[0], TAG_STATIC_FILE);

        let decoded = StaticFileRef::decompress(&compressed).unwrap();
        assert_eq!(decoded, ptr);
    }

    #[test]
    fn inline_roundtrip() {
        let data = vec![1, 2, 3, 4, 5];
        let inline = StaticFileRef::inline(data.clone());
        let compressed = inline.clone().compress().unwrap();
        assert_eq!(compressed.len(), 1 + data.len());
        assert_eq!(compressed[0], TAG_INLINE);

        let decoded = StaticFileRef::decompress(&compressed).unwrap();
        assert_eq!(decoded, inline);
    }

    #[test]
    fn empty_bytes_error() {
        assert!(StaticFileRef::decompress(&[]).is_err());
    }
}
