use bonsai_trie::{
    trie::trees::{FullMerkleTrees, PartialMerkleTrees},
    BonsaiDatabase, BonsaiPersistentDatabase, MultiProof,
};
use katana_primitives::block::BlockNumber;
use katana_primitives::contract::{StorageKey, StorageValue};
use katana_primitives::hash::Pedersen;
use katana_primitives::{ContractAddress, Felt};

use crate::id::CommitId;

pub struct StoragesTrie<DB: BonsaiDatabase, TreeType = FullMerkleTrees<Pedersen, DB, CommitId>> {
    /// The contract address the storage trie belongs to.
    address: ContractAddress,
    trie: crate::BonsaiTrie<DB, Pedersen, TreeType>,
}

pub type PartialStoragesTrie<DB> = StoragesTrie<DB, PartialMerkleTrees<Pedersen, DB, CommitId>>;

impl<DB: BonsaiDatabase> StoragesTrie<DB> {
    pub fn new(db: DB, address: ContractAddress) -> Self {
        Self { address, trie: crate::BonsaiTrie::new(db) }
    }

    pub fn root(&self) -> Felt {
        self.trie.root(&self.address.to_bytes_be())
    }

    pub fn multiproof(&mut self, storage_keys: Vec<StorageKey>) -> MultiProof {
        self.trie.multiproof(&self.address.to_bytes_be(), storage_keys)
    }
}

impl<DB> StoragesTrie<DB>
where
    DB: BonsaiDatabase + BonsaiPersistentDatabase<CommitId>,
{
    pub fn insert(&mut self, storage_key: StorageKey, storage_value: StorageValue) {
        self.trie.insert(&self.address.to_bytes_be(), storage_key, storage_value)
    }

    pub fn commit(&mut self, block: BlockNumber) {
        self.trie.commit(block.into())
    }
}

impl<DB: BonsaiDatabase, TreeType> std::fmt::Debug for StoragesTrie<DB, TreeType> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoragesTrie")
            .field("address", &self.address)
            .field("trie", &"<BonsaiTrie>")
            .finish()
    }
}

impl<DB: BonsaiDatabase> PartialStoragesTrie<DB> {
    pub fn new_partial(db: DB, address: ContractAddress) -> Self {
        Self { address, trie: crate::PartialBonsaiTrie::new_partial(db) }
    }

    pub fn root(&self) -> Felt {
        self.trie.root(&self.address.to_bytes_be())
    }

    pub fn multiproof(&mut self, storage_keys: Vec<StorageKey>) -> MultiProof {
        self.trie.multiproof(&self.address.to_bytes_be(), storage_keys)
    }
}

impl<DB> PartialStoragesTrie<DB>
where
    DB: BonsaiDatabase + BonsaiPersistentDatabase<CommitId>,
{
    pub fn insert(
        &mut self,
        storage_key: StorageKey,
        storage_value: StorageValue,
        proof: MultiProof,
        original_root: Felt,
    ) {
        self.trie.insert(
            &self.address.to_bytes_be(),
            storage_key,
            storage_value,
            proof,
            original_root,
        )
    }

    pub fn commit(&mut self, block: BlockNumber) {
        self.trie.commit(block.into())
    }
}
