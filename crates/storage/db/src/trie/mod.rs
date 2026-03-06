use katana_primitives::block::BlockNumber;
use katana_primitives::{ContractAddress, Felt};
use katana_trie::bitvec::order::Msb0;
use katana_trie::bitvec::slice::BitSlice;
use katana_trie::node::{Node, NodeRef, StoredNode, TrieNodeIndex, TrieUpdate};
use katana_trie::{ClassesTrie, ContractsTrie, StoragesTrie};

use crate::abstraction::{DbTx, DbTxMut};
use crate::models::list::BlockList;
use crate::models::trie::{TrieNodeEntry, TrieType};
use crate::tables;

/// Composite key for TrieRoots and TrieBlockLog tables.
/// Encodes trie type in the upper 8 bits and block number in the lower 56 bits.
fn trie_composite_key(trie_type: TrieType, block_number: BlockNumber) -> u64 {
    ((trie_type as u64) << 56) | block_number
}

/// Factory for creating trie instances backed by the database.
#[derive(Debug)]
pub struct TrieDbFactory<Tx: DbTx> {
    tx: Tx,
}

impl<Tx: DbTx> TrieDbFactory<Tx> {
    pub fn new(tx: Tx) -> Self {
        Self { tx }
    }

    /// Returns the latest classes trie (using the most recent root).
    pub fn classes_trie(
        &self,
        block: BlockNumber,
    ) -> ClassesTrie<DbTrieStorage<tables::TrieClassNodes, tables::TrieClassLeaves, Tx>> {
        let storage = DbTrieStorage::new(self.tx.clone());
        match self.get_root(TrieType::Classes, block) {
            Some(root) => ClassesTrie::new(storage, root),
            None => ClassesTrie::empty(storage),
        }
    }

    /// Returns the latest contracts trie.
    pub fn contracts_trie(
        &self,
        block: BlockNumber,
    ) -> ContractsTrie<DbTrieStorage<tables::TrieContractNodes, tables::TrieContractLeaves, Tx>>
    {
        let storage = DbTrieStorage::new(self.tx.clone());
        match self.get_root(TrieType::Contracts, block) {
            Some(root) => ContractsTrie::new(storage, root),
            None => ContractsTrie::empty(storage),
        }
    }

    /// Returns the storage trie for a given contract address.
    pub fn storages_trie(
        &self,
        address: ContractAddress,
        block: BlockNumber,
    ) -> StoragesTrie<DbTrieStorage<tables::TrieStorageNodes, tables::TrieStorageLeaves, Tx>> {
        let storage = DbTrieStorage::with_leaf_prefix(self.tx.clone(), Felt::from(address));
        match self.get_storage_root(address, block) {
            Some(root) => StoragesTrie::new(storage, address, root),
            None => StoragesTrie::empty(storage, address),
        }
    }

    /// Public accessor for getting the root index for proof generation.
    pub fn get_root_for_proofs(
        &self,
        trie_type: TrieType,
        block: BlockNumber,
    ) -> Option<TrieNodeIndex> {
        self.get_root(trie_type, block)
    }

    /// Public accessor for getting a storage root index for proof generation.
    pub fn get_storage_root_for_proofs(
        &self,
        address: ContractAddress,
        block: BlockNumber,
    ) -> Option<TrieNodeIndex> {
        self.get_storage_root(address, block)
    }

    /// Looks up the root TrieNodeIndex for the given trie type and block.
    fn get_root(&self, trie_type: TrieType, block: BlockNumber) -> Option<TrieNodeIndex> {
        let key = trie_composite_key(trie_type, block);
        self.tx.get::<tables::TrieRoots>(key).ok().flatten().map(TrieNodeIndex)
    }

    /// Looks up the root TrieNodeIndex for a contract's storage trie at a block.
    /// Storage tries use a different key scheme combining the contract address.
    fn get_storage_root(
        &self,
        address: ContractAddress,
        block: BlockNumber,
    ) -> Option<TrieNodeIndex> {
        // For storage tries, we encode the address hash + block in the key
        // We use the lower 28 bytes of the address hash + trie_type + block
        let addr_hash = {
            let bytes = Felt::from(address).to_bytes_be();
            u64::from_be_bytes(bytes[24..32].try_into().unwrap())
        };
        // Use a different discriminator for per-contract storage tries
        let key = (TrieType::Storage as u64) << 56 | (addr_hash & 0x00FFFFFF_FFFFFFFF);
        // Actually, storage roots need a different approach since we can have many contracts.
        // Let's use a simple scheme: look up from TrieRoots with a contract-specific key.
        // For now, we'll use a separate table or encode block+address differently.
        //
        // Simpler approach: use the TrieRoots table with key = hash(address, block)
        // But that doesn't allow efficient lookup. Let's use a separate per-address scheme.
        //
        // For the current implementation, storage roots are stored per (address, block)
        // using a deterministic u64 key derived from both.
        let _ = key;
        let storage_root_key = storage_trie_root_key(address, block);
        self.tx.get::<tables::TrieRoots>(storage_root_key).ok().flatten().map(TrieNodeIndex)
    }
}

/// Generates a deterministic key for storage trie roots.
/// Combines contract address and block number into a u64 key.
/// Uses a different key space (high byte = 0x80+) to avoid collisions with class/contract tries.
fn storage_trie_root_key(address: ContractAddress, block: BlockNumber) -> u64 {
    // Use FNV-like mixing to combine address and block into a unique key.
    // High byte 0x80 distinguishes from TrieType keys (0x00, 0x01, 0x02).
    let addr_bytes = Felt::from(address).to_bytes_be();
    let addr_hash = u64::from_be_bytes(addr_bytes[24..32].try_into().unwrap());
    // Mix address and block: use XOR with golden ratio
    let mixed = addr_hash.wrapping_mul(0x517cc1b727220a95) ^ block;
    0x80_00000000000000 | (mixed & 0x00FFFFFFFFFFFFFF)
}

/// Default capacity for the node cache (number of entries).
const NODE_CACHE_CAPACITY: usize = 4096;

/// DB-backed storage implementation for the trie.
///
/// Includes an LRU cache for node entries to avoid redundant DB reads.
/// Both `get()` and `hash()` read from the same underlying `TrieNodeEntry`,
/// so caching the full entry serves both lookups. The cache uses an LRU
/// eviction policy to bound memory usage.
pub struct DbTrieStorage<Tb, Lb, Tx>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Lb: tables::Table<Key = Felt, Value = Felt>,
    Tx: DbTx,
{
    tx: Tx,
    /// Optional prefix for leaf key lookups (used for per-contract storage tries).
    leaf_key_prefix: Option<Felt>,
    /// LRU cache for trie node entries, keyed by node index.
    node_cache: std::cell::RefCell<quick_cache::unsync::Cache<u64, TrieNodeEntry>>,
    _phantom: std::marker::PhantomData<(Tb, Lb)>,
}

impl<Tb, Lb, Tx> std::fmt::Debug for DbTrieStorage<Tb, Lb, Tx>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Lb: tables::Table<Key = Felt, Value = Felt>,
    Tx: DbTx,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cache = self.node_cache.borrow();
        f.debug_struct("DbTrieStorage")
            .field("tx", &"..")
            .field("cache_len", &cache.len())
            .field("cache_capacity", &cache.capacity())
            .finish()
    }
}

impl<Tb, Lb, Tx> DbTrieStorage<Tb, Lb, Tx>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Lb: tables::Table<Key = Felt, Value = Felt>,
    Tx: DbTx,
{
    pub fn new(tx: Tx) -> Self {
        Self {
            tx,
            leaf_key_prefix: None,
            node_cache: std::cell::RefCell::new(quick_cache::unsync::Cache::new(
                NODE_CACHE_CAPACITY,
            )),
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_leaf_prefix(tx: Tx, prefix: Felt) -> Self {
        Self {
            tx,
            leaf_key_prefix: Some(prefix),
            node_cache: std::cell::RefCell::new(quick_cache::unsync::Cache::new(
                NODE_CACHE_CAPACITY,
            )),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Fetches a node entry, returning a cached copy if available.
    /// Uses LRU eviction when the cache is full.
    fn get_cached_entry(&self, index: TrieNodeIndex) -> anyhow::Result<Option<TrieNodeEntry>> {
        // Check cache first
        if let Some(entry) = self.node_cache.borrow_mut().get(&index.0) {
            return Ok(Some(entry.clone()));
        }

        // Cache miss — read from DB
        let entry = self
            .tx
            .get::<Tb>(index.0)
            .map_err(|e| anyhow::anyhow!("DB error reading trie node {}: {e}", index.0))?;

        if let Some(ref entry) = entry {
            self.node_cache.borrow_mut().insert(index.0, entry.clone());
        }

        Ok(entry)
    }
}

/// Converts a trie path (BitSlice) back to a Felt key for leaf table lookup.
fn path_to_felt(path: &BitSlice<u8, Msb0>) -> Felt {
    use katana_trie::bitvec::prelude::BitVec;
    let mut padded = BitVec::<u8, Msb0>::repeat(false, 256 - path.len());
    padded.extend_from_bitslice(path);
    let bytes: [u8; 32] = padded.as_raw_slice().try_into().unwrap_or([0u8; 32]);
    Felt::from_bytes_be(&bytes)
}

/// Computes a composite leaf key for storage tries by combining address + key.
/// Uses Pedersen hash via katana-trie to create a unique composite key.
fn storage_leaf_key(address: Felt, key: Felt) -> Felt {
    katana_trie::pedersen_hash(&address, &key)
}

impl<Tb, Lb, Tx> katana_trie::Storage for DbTrieStorage<Tb, Lb, Tx>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Lb: tables::Table<Key = Felt, Value = Felt>,
    Tx: DbTx,
{
    fn get(&self, index: TrieNodeIndex) -> anyhow::Result<Option<StoredNode>> {
        let entry = self.get_cached_entry(index)?;
        Ok(entry.map(|e| e.node.clone()))
    }

    fn hash(&self, index: TrieNodeIndex) -> anyhow::Result<Option<Felt>> {
        let entry = self.get_cached_entry(index)?;
        Ok(entry.map(|e| e.hash))
    }

    fn leaf(&self, path: &BitSlice<u8, Msb0>) -> anyhow::Result<Option<Felt>> {
        let key = path_to_felt(path);
        let db_key = if let Some(prefix) = self.leaf_key_prefix {
            storage_leaf_key(prefix, key)
        } else {
            key
        };
        let result = self
            .tx
            .get::<Lb>(db_key)
            .map_err(|e| anyhow::anyhow!("DB error reading trie leaf: {e}"))?;
        Ok(result)
    }
}

/// Persists a TrieUpdate to the database.
pub fn persist_trie_update<Tb, Lb, Tx>(
    tx: &Tx,
    update: &TrieUpdate,
    block: BlockNumber,
    trie_type: TrieType,
    next_index: &mut u64,
) -> anyhow::Result<Option<TrieNodeIndex>>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Lb: tables::Table<Key = Felt, Value = Felt>,
    Tx: DbTxMut,
{
    if update.nodes_added.is_empty() {
        return Ok(None);
    }

    let base_index = *next_index;
    let mut added_indices = BlockList::default();

    for (i, (hash, node)) in update.nodes_added.iter().enumerate() {
        let index = base_index + i as u64;
        let stored = resolve_node_ref(node, base_index);
        let entry = TrieNodeEntry { hash: *hash, node: stored };
        tx.put::<Tb>(index, entry)
            .map_err(|e| anyhow::anyhow!("DB error writing trie node {index}: {e}"))?;
        added_indices.insert(index);
    }

    // Persist leaf values
    for (path, value) in &update.leaves {
        let key = path_to_felt(path);
        tx.put::<Lb>(key, *value)
            .map_err(|e| anyhow::anyhow!("DB error writing trie leaf: {e}"))?;
    }

    *next_index = base_index + update.nodes_added.len() as u64;

    // Store the root index
    let root_index = base_index + update.nodes_added.len() as u64 - 1;

    // Store root in TrieRoots
    let root_key = trie_composite_key(trie_type, block);
    tx.put::<tables::TrieRoots>(root_key, root_index)
        .map_err(|e| anyhow::anyhow!("DB error writing trie root: {e}"))?;

    // Store block log for revert support
    let log_key = trie_composite_key(trie_type, block);
    tx.put::<tables::TrieBlockLog>(log_key, added_indices)
        .map_err(|e| anyhow::anyhow!("DB error writing trie block log: {e}"))?;

    Ok(Some(TrieNodeIndex(root_index)))
}

/// Persists a storage trie update for a specific contract.
pub fn persist_storage_trie_update<Tx>(
    tx: &Tx,
    update: &TrieUpdate,
    block: BlockNumber,
    address: ContractAddress,
    next_index: &mut u64,
) -> anyhow::Result<Option<TrieNodeIndex>>
where
    Tx: DbTxMut,
{
    if update.nodes_added.is_empty() {
        return Ok(None);
    }

    let base_index = *next_index;
    let mut added_indices = BlockList::default();

    for (i, (hash, node)) in update.nodes_added.iter().enumerate() {
        let index = base_index + i as u64;
        let stored = resolve_node_ref(node, base_index);
        let entry = TrieNodeEntry { hash: *hash, node: stored };
        tx.put::<tables::TrieStorageNodes>(index, entry)
            .map_err(|e| anyhow::anyhow!("DB error writing storage trie node {index}: {e}"))?;
        added_indices.insert(index);
    }

    // Persist leaf values with address-prefixed keys
    let addr_felt = Felt::from(address);
    for (path, value) in &update.leaves {
        let key = storage_leaf_key(addr_felt, path_to_felt(path));
        tx.put::<tables::TrieStorageLeaves>(key, *value)
            .map_err(|e| anyhow::anyhow!("DB error writing storage trie leaf: {e}"))?;
    }

    *next_index = base_index + update.nodes_added.len() as u64;
    let root_index = base_index + update.nodes_added.len() as u64 - 1;

    // Store root for this contract's storage trie
    let root_key = storage_trie_root_key(address, block);
    tx.put::<tables::TrieRoots>(root_key, root_index)
        .map_err(|e| anyhow::anyhow!("DB error writing storage trie root: {e}"))?;

    // Block log for storage tries - use the storage trie root key
    tx.put::<tables::TrieBlockLog>(root_key, added_indices)
        .map_err(|e| anyhow::anyhow!("DB error writing storage trie block log: {e}"))?;

    Ok(Some(TrieNodeIndex(root_index)))
}

/// Resolves a Node with NodeRef children into a StoredNode with concrete indices.
fn resolve_node_ref(node: &Node, base: u64) -> StoredNode {
    fn resolve_ref(r: &NodeRef, base: u64) -> TrieNodeIndex {
        match r {
            NodeRef::StorageIndex(idx) => *idx,
            NodeRef::Index(i) => TrieNodeIndex(base + *i as u64),
        }
    }

    match node {
        Node::Binary { left, right } => {
            StoredNode::Binary { left: resolve_ref(left, base), right: resolve_ref(right, base) }
        }
        Node::Edge { child, path } => {
            StoredNode::Edge { child: resolve_ref(child, base), path: path.clone() }
        }
        Node::LeafBinary => StoredNode::LeafBinary,
        Node::LeafEdge { path } => StoredNode::LeafEdge { path: path.clone() },
    }
}

/// Gets the next available node index for a trie node table.
pub fn next_node_index<Tb, Tx>(tx: &Tx) -> anyhow::Result<u64>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Tx: DbTx,
{
    use crate::abstraction::DbCursor;

    let mut cursor =
        tx.cursor::<Tb>().map_err(|e| anyhow::anyhow!("DB error creating cursor: {e}"))?;
    match cursor.last().map_err(|e| anyhow::anyhow!("DB error seeking last: {e}"))? {
        Some((key, _)) => Ok(key + 1),
        None => Ok(0),
    }
}

/// Reverts trie state to a target block by removing nodes added after it.
pub fn revert_trie_to_block<Tx: DbTxMut>(
    tx: &Tx,
    target_block: BlockNumber,
    latest_block: BlockNumber,
) -> anyhow::Result<()> {
    for block in (target_block + 1)..=latest_block {
        // Revert class trie
        revert_trie_block::<tables::TrieClassNodes, Tx>(
            tx,
            trie_composite_key(TrieType::Classes, block),
        )?;

        // Revert contract trie
        revert_trie_block::<tables::TrieContractNodes, Tx>(
            tx,
            trie_composite_key(TrieType::Contracts, block),
        )?;

        // Note: storage trie reverts would need to iterate over all contracts
        // that had storage changes at this block. For now, we handle class and contract tries.
        // Storage trie revert is more complex and would require tracking which contracts
        // were modified at each block.
    }

    Ok(())
}

fn revert_trie_block<Tb, Tx>(tx: &Tx, log_key: u64) -> anyhow::Result<()>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Tx: DbTxMut,
{
    // Read block log to find added node indices
    if let Some(added) = tx
        .get::<tables::TrieBlockLog>(log_key)
        .map_err(|e| anyhow::anyhow!("DB error reading block log: {e}"))?
    {
        // Delete all nodes added in this block
        for index in added.iter() {
            let _ = tx.delete::<Tb>(index, None);
        }

        // Delete root and block log entries
        let _ = tx.delete::<tables::TrieRoots>(log_key, None);
        let _ = tx.delete::<tables::TrieBlockLog>(log_key, None);
    }

    Ok(())
}

/// Prunes trie data for a block (removes roots and block log, but not nodes).
pub fn prune_trie_block<Tx: DbTxMut>(tx: &Tx, block: BlockNumber) -> anyhow::Result<()> {
    // Remove roots and block logs for all trie types
    for trie_type in [TrieType::Classes, TrieType::Contracts] {
        let key = trie_composite_key(trie_type, block);
        let _ = tx.delete::<tables::TrieRoots>(key, None);
        let _ = tx.delete::<tables::TrieBlockLog>(key, None);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abstraction::{Database, DbTxMut as _};
    use crate::mdbx::test_utils;

    #[test]
    fn test_path_to_felt_roundtrip() {
        use katana_trie::bitvec::prelude::*;
        use katana_trie::bitvec::view::AsBits;

        // Test that path_to_felt roundtrips correctly for various felts
        for i in [1u64, 42, 1000, u64::MAX] {
            let felt = Felt::from(i);
            let bytes = felt.to_bytes_be();
            let path = bytes.as_bits::<Msb0>()[5..].to_owned();
            assert_eq!(path.len(), 251);
            let reconstructed = path_to_felt(&path);
            // Top 5 bits are zeroed, but for small values they're already zero
            let expected = {
                let mut bytes = felt.to_bytes_be();
                bytes[0] &= 0x07; // clear top 5 bits
                Felt::from_bytes_be(&bytes)
            };
            assert_eq!(
                reconstructed, expected,
                "path_to_felt roundtrip failed for {i}: got {:#x}, expected {:#x}",
                reconstructed, expected
            );
        }

        // Test with a value that has top bits set (bits 3-4 are 1)
        let felt = Felt::from_hex_unchecked(
            "0x07ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        );
        let bytes = felt.to_bytes_be();
        let path = bytes.as_bits::<Msb0>()[5..].to_owned();
        let reconstructed = path_to_felt(&path);
        assert_eq!(reconstructed, felt, "full felt roundtrip failed");
    }

    #[test]
    fn test_classes_trie_multi_block_with_db() {
        let db = test_utils::create_test_db();

        // Block 0: insert some classes
        {
            let tx = db.tx_mut().unwrap();
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0); // empty, no root at block 0

            trie.insert(Felt::from(1u64), Felt::from(100u64)).unwrap();
            trie.insert(Felt::from(2u64), Felt::from(200u64)).unwrap();

            let update = trie.commit().unwrap();
            assert_ne!(update.root_commitment, Felt::ZERO);
            assert!(!update.leaves.is_empty(), "leaves should be populated in TrieUpdate");

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();

            tx.commit().unwrap();
        }

        // Block 1: insert more classes, building on block 0's root
        {
            let tx = db.tx_mut().unwrap();
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0); // load block 0's root

            // This should work — leaf values from block 0 are in TrieClassLeaves
            trie.insert(Felt::from(3u64), Felt::from(300u64)).unwrap();

            let update = trie.commit().unwrap();
            assert_ne!(update.root_commitment, Felt::ZERO);

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
                &tx,
                &update,
                1,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();

            tx.commit().unwrap();
        }
    }

    #[test]
    fn test_classes_trie_multi_block_single_tx() {
        use katana_primitives::class::{ClassHash, CompiledClassHash};

        let db = test_utils::create_test_db();
        let tx = db.tx_mut().unwrap();

        // Block 0: insert 10 classes with spread-out hashes
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            for i in 0u64..10 {
                let class_hash = ClassHash::from(Felt::from(i * 7919 + 1000)); // spread values
                let compiled_hash = CompiledClassHash::from(Felt::from(i * 100 + 100));
                trie.insert(class_hash, compiled_hash).unwrap();
            }

            let update = trie.commit().unwrap();

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

        // Block 1: insert 10 more classes within same tx
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            for i in 0u64..10 {
                let class_hash = ClassHash::from(Felt::from(i * 7919 + 100000));
                let compiled_hash = CompiledClassHash::from(Felt::from(i * 100 + 10000));
                trie.insert(class_hash, compiled_hash).unwrap();
            }

            let update = trie.commit().unwrap();

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
                &tx,
                &update,
                1,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

        tx.commit().unwrap();
    }

    #[test]
    fn test_classes_trie_2keys_multi_block() {
        use katana_primitives::class::{ClassHash, CompiledClassHash};

        let db = test_utils::create_test_db();
        let tx = db.tx_mut().unwrap();

        // Block 0: insert 2 classes
        let root0;
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            trie.insert(
                ClassHash::from(Felt::from_hex_unchecked(
                    "0x0000000000000000000000000000000071f11b9b21ba14a3935fcf8dbf110022",
                )),
                CompiledClassHash::from(Felt::from(1u64)),
            )
            .unwrap();
            trie.insert(
                ClassHash::from(Felt::from_hex_unchecked(
                    "0x000000000000000000000000000000005555555555555555aaaaaaaaaaaaaaaa",
                )),
                CompiledClassHash::from(Felt::from(2u64)),
            )
            .unwrap();

            let update = trie.commit().unwrap();
            root0 = update.root_commitment;

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

        // Block 1: insert 1 more class
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            trie.insert(
                ClassHash::from(Felt::from_hex_unchecked(
                    "0x000000000000000000000000000000003333333333333333cccccccccccccccc",
                )),
                CompiledClassHash::from(Felt::from(3u64)),
            )
            .unwrap();

            let update = trie.commit().unwrap();
            assert_ne!(update.root_commitment, Felt::ZERO);
            assert_ne!(update.root_commitment, root0);
        }

        tx.commit().unwrap();
    }

    #[test]
    fn test_classes_trie_multi_block_random_keys() {
        use katana_primitives::class::{ClassHash, CompiledClassHash};

        let db = test_utils::create_test_db();
        let tx = db.tx_mut().unwrap();

        // Generate pseudo-random keys using a simple LCG
        let mut rng_state = 0x12345678u64;
        let mut next_random_felt = || -> Felt {
            // Simple LCG: x_{n+1} = (a * x_n + c) mod m
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let hi = rng_state;
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let lo = rng_state;
            // Create a Felt from two u64s
            let mut bytes = [0u8; 32];
            bytes[16..24].copy_from_slice(&hi.to_be_bytes());
            bytes[24..32].copy_from_slice(&lo.to_be_bytes());
            // Ensure it's a valid Felt (top 5 bits zero)
            bytes[0] &= 0x07;
            Felt::from_bytes_be(&bytes)
        };

        // Block 0: insert 10 classes with random hashes
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            for _ in 0u64..10 {
                let class_hash = ClassHash::from(next_random_felt());
                let compiled_hash = CompiledClassHash::from(next_random_felt());
                trie.insert(class_hash, compiled_hash).unwrap();
            }

            let update = trie.commit().unwrap();

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

        // Block 1: insert 10 more classes with random hashes
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            for _ in 0u64..10 {
                let class_hash = ClassHash::from(next_random_felt());
                let compiled_hash = CompiledClassHash::from(next_random_felt());
                trie.insert(class_hash, compiled_hash).unwrap();
            }

            let update = trie.commit().unwrap();

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
                &tx,
                &update,
                1,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

        tx.commit().unwrap();
    }
}
