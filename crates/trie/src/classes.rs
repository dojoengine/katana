use crate::id::CommitId;
use bonsai_trie::{BonsaiDatabase, BonsaiPersistentDatabase, MultiProof};
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::hash::{Poseidon, StarkHash};
use katana_primitives::Felt;
use starknet::macros::short_string;
#[derive(Debug)]
pub struct ClassesMultiProof(pub MultiProof);

impl ClassesMultiProof {
    // TODO: maybe perform results check in this method as well. make it accept the compiled class
    // hashes
    pub fn verify(&self, root: Felt, class_hashes: Vec<ClassHash>) -> Vec<Felt> {
        crate::verify_proof::<Poseidon>(&self.0, root, class_hashes)
    }
}

impl From<MultiProof> for ClassesMultiProof {
    fn from(value: MultiProof) -> Self {
        Self(value)
    }
}

pub struct ClassesTrie<
    DB: BonsaiDatabase,
    TreeType = bonsai_trie::trie::trees::FullMerkleTrees<Poseidon, DB, CommitId>,
> {
    trie: crate::BonsaiTrie<DB, Poseidon, TreeType>,
}
type PartialClassesTrie<DB> =
    ClassesTrie<DB, bonsai_trie::trie::trees::PartialMerkleTrees<Poseidon, DB, CommitId>>;

//////////////////////////////////////////////////////////////
// 	ClassesTrie implementations
//////////////////////////////////////////////////////////////

impl<DB: BonsaiDatabase> ClassesTrie<DB> {
    const BONSAI_IDENTIFIER: &'static [u8] = b"classes";

    pub fn new(db: DB) -> Self {
        Self { trie: crate::BonsaiTrie::new(db) }
    }

    pub fn root(&self) -> Felt {
        self.trie.root(Self::BONSAI_IDENTIFIER)
    }

    pub fn multiproof(&mut self, class_hashes: Vec<ClassHash>) -> MultiProof {
        self.trie.multiproof(Self::BONSAI_IDENTIFIER, class_hashes)
    }
}

impl<DB: BonsaiDatabase> PartialClassesTrie<DB> {
    const BONSAI_IDENTIFIER: &'static [u8] = b"classes";

    pub fn new_partial(db: DB) -> Self {
        Self { trie: crate::PartialBonsaiTrie::new_partial(db) }
    }

    pub fn root(&self) -> Felt {
        self.trie.root(Self::BONSAI_IDENTIFIER)
    }
}

impl<DB> ClassesTrie<DB>
where
    DB: BonsaiDatabase + BonsaiPersistentDatabase<CommitId>,
{
    pub fn insert(&mut self, hash: ClassHash, compiled_hash: CompiledClassHash) {
        let value = compute_classes_trie_value(compiled_hash);
        self.trie.insert(Self::BONSAI_IDENTIFIER, hash, value)
    }

    pub fn commit(&mut self, block: BlockNumber) {
        self.trie.commit(block.into())
    }
}

impl<DB> PartialClassesTrie<DB>
where
    DB: BonsaiDatabase + BonsaiPersistentDatabase<CommitId>,
{
    pub fn insert(
        &mut self,
        hash: ClassHash,
        compiled_hash: CompiledClassHash,
        proof: MultiProof,
        original_root: Felt,
    ) {
        let value = compute_classes_trie_value(compiled_hash);
        self.trie.insert(Self::BONSAI_IDENTIFIER, hash, value, proof, original_root)
    }

    pub fn commit(&mut self, block: BlockNumber) {
        self.trie.commit(block.into())
    }
}

pub fn compute_classes_trie_value(compiled_class_hash: CompiledClassHash) -> Felt {
    // https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#classes_trie
    const CONTRACT_CLASS_LEAF_V0: Felt = short_string!("CONTRACT_CLASS_LEAF_V0");
    Poseidon::hash(&CONTRACT_CLASS_LEAF_V0, &compiled_class_hash)
}
