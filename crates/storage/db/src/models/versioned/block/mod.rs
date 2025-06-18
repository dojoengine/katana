use katana_primitives::block::{self, Header};
use serde::{Deserialize, Serialize};

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

mod v6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub enum VersionedHeader {
    V6(v6::Header),
    V7(Header),
}

impl Default for VersionedHeader {
    fn default() -> Self {
        Self::V7(Default::default())
    }
}

impl From<block::Header> for VersionedHeader {
    fn from(header: block::Header) -> Self {
        Self::V7(header)
    }
}

impl From<VersionedHeader> for block::Header {
    fn from(versioned: VersionedHeader) -> Self {
        match versioned {
            VersionedHeader::V7(header) => header,
            VersionedHeader::V6(header) => header.into(),
        }
    }
}

impl Compress for VersionedHeader {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        postcard::to_stdvec(&self).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for VersionedHeader {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();

        if let Ok(header) = postcard::from_bytes::<Self>(bytes) {
            return Ok(header);
        }

        // Try deserializing as V7 first, then fall back to V6
        if let Ok(header) = postcard::from_bytes::<Header>(bytes) {
            return Ok(VersionedHeader::V7(header));
        }

        if let Ok(header) = postcard::from_bytes::<v6::Header>(bytes) {
            return Ok(VersionedHeader::V6(header));
        }

        Err(CodecError::Decompress("failed to deserialize header: unknown format".to_string()))
    }
}
