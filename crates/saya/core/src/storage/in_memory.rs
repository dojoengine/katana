use crate::storage::{BlockWithDa, ChainHead, StorageBackend};

/// An entirely in-memory storage backend useful for development and testing purposes.
#[derive(Default)]
pub struct InMemoryStorageBackend {
    last_block: Option<BlockWithDa>,
}

impl InMemoryStorageBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StorageBackend for InMemoryStorageBackend {
    async fn get_chain_head(&self) -> ChainHead {
        match self.last_block {
            Some(last_block) => ChainHead::Block(last_block),
            None => ChainHead::Genesis,
        }
    }

    async fn set_chain_head(&mut self, block: BlockWithDa) {
        self.last_block = Some(block);
    }
}
