use katana_primitives::class::{ClassHash, CompiledClass, CompiledClassHash};
use serde::{Deserialize, Serialize};

use crate::codecs::{Compress, Decode, Decompress, Encode};
use crate::error::CodecError;

/// The value for the [MigratedCompiledClassHashes](crate::tables::MigratedCompiledClassHashes)
/// table.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MigratedCompiledClassHash {
    pub class_hash: ClassHash,
    pub compiled_class_hash: CompiledClassHash,
}

impl Compress for MigratedCompiledClassHash {
    type Compressed = Vec<u8>;

    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.class_hash.encode());
        buf.extend_from_slice(&self.compiled_class_hash.compress()?);
        Ok(buf)
    }
}

impl Decompress for MigratedCompiledClassHash {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();
        let class_hash = ClassHash::decode(&bytes[0..32])?;
        let compiled_class_hash = ClassHash::decompress(&bytes[32..])?;
        Ok(Self { class_hash, compiled_class_hash })
    }
}

impl Compress for CompiledClass {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        serde_json::to_vec(&self).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for CompiledClass {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        serde_json::from_slice(bytes.as_ref()).map_err(|e| CodecError::Decode(e.to_string()))
    }
}
