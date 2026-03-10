use katana_primitives::state::StateUpdates;
use serde::{Deserialize, Serialize};

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VersionedStateUpdates {
    V1(StateUpdates),
}

impl From<StateUpdates> for VersionedStateUpdates {
    fn from(state_updates: StateUpdates) -> Self {
        Self::V1(state_updates)
    }
}

impl From<VersionedStateUpdates> for StateUpdates {
    fn from(versioned: VersionedStateUpdates) -> Self {
        match versioned {
            VersionedStateUpdates::V1(state_updates) => state_updates,
        }
    }
}

impl Default for VersionedStateUpdates {
    fn default() -> Self {
        Self::V1(StateUpdates::default())
    }
}

impl Compress for VersionedStateUpdates {
    type Compressed = Vec<u8>;

    fn compress(self) -> Result<Self::Compressed, CodecError> {
        postcard::to_stdvec(&self).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for VersionedStateUpdates {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();

        if let Ok(state_updates) = postcard::from_bytes::<Self>(bytes) {
            return Ok(state_updates);
        }

        if let Ok(state_updates) = postcard::from_bytes::<StateUpdates>(bytes) {
            return Ok(Self::V1(state_updates));
        }

        Err(CodecError::Decompress(
            "failed to deserialize versioned state updates: unknown format".to_string(),
        ))
    }
}
