use serde::{Deserialize, Serialize};

mod v7;

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

pub type CurrentContractClass = katana_primitives::class::ContractClass;
pub type V8ContractClass = CurrentContractClass;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VersionedContractClass {
    V7(v7::ContractClass),
    V8(V8ContractClass),
}

impl From<CurrentContractClass> for VersionedContractClass {
    fn from(class: CurrentContractClass) -> Self {
        Self::V8(class)
    }
}

impl From<VersionedContractClass> for CurrentContractClass {
    fn from(versioned: VersionedContractClass) -> Self {
        match versioned {
            VersionedContractClass::V8(class) => class,
            VersionedContractClass::V7(class) => class.into(),
        }
    }
}

impl Default for VersionedContractClass {
    fn default() -> Self {
        Self::V8(CurrentContractClass::Legacy(Default::default()))
    }
}

impl Compress for VersionedContractClass {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        serde_json::to_vec(&self).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for VersionedContractClass {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();

        if let Ok(class) = serde_json::from_slice::<Self>(bytes) {
            return Ok(class);
        }

        // Try deserializing as V8 (current) first, then fall back to V7
        if let Ok(class) = serde_json::from_slice::<V8ContractClass>(bytes) {
            return Ok(VersionedContractClass::V8(class));
        }

        if let Ok(class) = serde_json::from_slice::<v7::ContractClass>(bytes) {
            return Ok(VersionedContractClass::V7(class));
        }

        Err(CodecError::Decompress(
            "failed to deserialize contract class: unknown format".to_string(),
        ))
    }
}
