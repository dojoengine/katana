use katana_primitives::class::CompiledClass;

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

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
