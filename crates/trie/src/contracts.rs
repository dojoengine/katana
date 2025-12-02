use bonsai_trie::{
    trie::trees::{FullMerkleTrees, PartialMerkleTrees},
    BonsaiDatabase, BonsaiPersistentDatabase, MultiProof,
};
use katana_primitives::block::BlockNumber;
use katana_primitives::hash::Pedersen;
use katana_primitives::{ContractAddress, Felt};

use crate::id::CommitId;

const CONTRACTS_IDENTIFIER: &[u8] = b"contracts";

pub struct ContractsTrie<DB: BonsaiDatabase, TreeType = FullMerkleTrees<Pedersen, DB, CommitId>> {
    trie: crate::BonsaiTrie<DB, Pedersen, TreeType>,
}

pub type PartialContractsTrie<DB> = ContractsTrie<DB, PartialMerkleTrees<Pedersen, DB, CommitId>>;

//////////////////////////////////////////////////////////////
// 	ContractsTrie implementations
//////////////////////////////////////////////////////////////

impl<DB: BonsaiDatabase> ContractsTrie<DB> {
    pub fn new(db: DB) -> Self {
        Self { trie: crate::BonsaiTrie::new(db) }
    }

    pub fn root(&self) -> Felt {
        self.trie.root(CONTRACTS_IDENTIFIER)
    }

    pub fn multiproof(&mut self, addresses: Vec<ContractAddress>) -> MultiProof {
        let keys = addresses.into_iter().map(Felt::from).collect::<Vec<Felt>>();
        self.trie.multiproof(CONTRACTS_IDENTIFIER, keys)
    }
}

impl<DB> ContractsTrie<DB>
where
    DB: BonsaiDatabase + BonsaiPersistentDatabase<CommitId>,
{
    pub fn insert(&mut self, address: ContractAddress, state_hash: Felt) {
        self.trie.insert(CONTRACTS_IDENTIFIER, *address, state_hash)
    }

    pub fn commit(&mut self, block: BlockNumber) {
        self.trie.commit(block.into())
    }
}

impl<DB: BonsaiDatabase> PartialContractsTrie<DB> {
    pub fn new_partial(db: DB) -> Self {
        Self { trie: crate::PartialBonsaiTrie::new_partial(db) }
    }

    pub fn root(&self) -> Felt {
        self.trie.root(CONTRACTS_IDENTIFIER)
    }

    pub fn multiproof(&mut self, addresses: Vec<ContractAddress>) -> MultiProof {
        let keys = addresses.into_iter().map(Felt::from).collect::<Vec<Felt>>();
        self.trie.multiproof(CONTRACTS_IDENTIFIER, keys)
    }
}

impl<DB> PartialContractsTrie<DB>
where
    DB: BonsaiDatabase + BonsaiPersistentDatabase<CommitId>,
{
    pub fn insert(
        &mut self,
        address: ContractAddress,
        state_hash: Felt,
        proof: MultiProof,
        original_root: Felt,
    ) {
        self.trie.insert(CONTRACTS_IDENTIFIER, *address, state_hash, proof, original_root)
    }

    pub fn commit(&mut self, block: BlockNumber) {
        self.trie.commit(block.into())
    }
}

impl<DB: BonsaiDatabase, TreeType> std::fmt::Debug for ContractsTrie<DB, TreeType> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractsTrie").field("trie", &"<BonsaiTrie>").finish()
    }
}

#[derive(Debug, Default)]
pub struct ContractLeaf {
    pub class_hash: Option<Felt>,
    pub storage_root: Option<Felt>,
    pub nonce: Option<Felt>,
}
