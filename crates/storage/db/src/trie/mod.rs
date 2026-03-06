use std::collections::VecDeque;

use katana_primitives::block::BlockNumber;
use katana_primitives::{ContractAddress, Felt};
use katana_trie::node::{Node, NodeRef, StoredNode, TrieNodeIndex, TrieUpdate};
use katana_trie::{ClassesTrie, ContractsTrie, MemStorage, StoragesTrie};

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

    /// Returns the classes trie at the given block.
    pub fn classes_trie(
        &self,
        block: BlockNumber,
    ) -> ClassesTrie<DbTrieStorage<tables::TrieClassNodes, Tx>> {
        let storage = DbTrieStorage::new(self.tx.clone());
        match self.get_root(TrieType::Classes, block) {
            Some(root) => ClassesTrie::new(storage, root),
            None => ClassesTrie::empty(storage),
        }
    }

    /// Returns the contracts trie at the given block.
    pub fn contracts_trie(
        &self,
        block: BlockNumber,
    ) -> ContractsTrie<DbTrieStorage<tables::TrieContractNodes, Tx>> {
        let storage = DbTrieStorage::new(self.tx.clone());
        match self.get_root(TrieType::Contracts, block) {
            Some(root) => ContractsTrie::new(storage, root),
            None => ContractsTrie::empty(storage),
        }
    }

    /// Returns the storage trie for a given contract address at the given block.
    pub fn storages_trie(
        &self,
        address: ContractAddress,
        block: BlockNumber,
    ) -> StoragesTrie<DbTrieStorage<tables::TrieStorageNodes, Tx>> {
        let storage = DbTrieStorage::new(self.tx.clone());
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
    fn get_storage_root(
        &self,
        address: ContractAddress,
        block: BlockNumber,
    ) -> Option<TrieNodeIndex> {
        let storage_root_key = storage_trie_root_key(address, block);
        self.tx.get::<tables::TrieRoots>(storage_root_key).ok().flatten().map(TrieNodeIndex)
    }

    /// Returns the classes trie at the given block, loaded entirely into memory.
    pub fn classes_trie_in_memory(
        &self,
        block: BlockNumber,
    ) -> anyhow::Result<ClassesTrie<MemStorage>> {
        match self.get_root(TrieType::Classes, block) {
            Some(root) => {
                let mem = load_trie_to_memory::<tables::TrieClassNodes, _>(&self.tx, root)?;
                Ok(ClassesTrie::new(mem, root))
            }
            None => Ok(ClassesTrie::empty(MemStorage::new())),
        }
    }

    /// Returns the contracts trie at the given block, loaded entirely into memory.
    pub fn contracts_trie_in_memory(
        &self,
        block: BlockNumber,
    ) -> anyhow::Result<ContractsTrie<MemStorage>> {
        match self.get_root(TrieType::Contracts, block) {
            Some(root) => {
                let mem = load_trie_to_memory::<tables::TrieContractNodes, _>(&self.tx, root)?;
                Ok(ContractsTrie::new(mem, root))
            }
            None => Ok(ContractsTrie::empty(MemStorage::new())),
        }
    }

    /// Returns the storage trie for a contract at the given block, loaded entirely into memory.
    pub fn storages_trie_in_memory(
        &self,
        address: ContractAddress,
        block: BlockNumber,
    ) -> anyhow::Result<StoragesTrie<MemStorage>> {
        match self.get_storage_root(address, block) {
            Some(root) => {
                let mem = load_trie_to_memory::<tables::TrieStorageNodes, _>(&self.tx, root)?;
                Ok(StoragesTrie::new(mem, address, root))
            }
            None => Ok(StoragesTrie::empty(MemStorage::new(), address)),
        }
    }
}

/// Loads all reachable nodes from a trie root into a [`MemStorage`] via BFS.
pub fn load_trie_to_memory<Tb, Tx>(tx: &Tx, root: TrieNodeIndex) -> anyhow::Result<MemStorage>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Tx: DbTx,
{
    let mut mem = MemStorage::new();
    let mut queue = VecDeque::new();
    queue.push_back(root);

    while let Some(index) = queue.pop_front() {
        let entry = tx
            .get::<Tb>(index.0)
            .map_err(|e| anyhow::anyhow!("DB error reading trie node {}: {e}", index.0))?
            .ok_or_else(|| anyhow::anyhow!("Trie node {} is missing from DB", index.0))?;

        // Enqueue children for non-leaf nodes
        match &entry.node {
            StoredNode::Binary { left, right } => {
                queue.push_back(*left);
                queue.push_back(*right);
            }
            StoredNode::Edge { child, .. } => {
                queue.push_back(*child);
            }
            StoredNode::LeafBinary { .. } | StoredNode::LeafEdge { .. } => {
                // Terminal nodes — no children to follow
            }
        }

        mem.insert_node(index.0, entry.hash, entry.node);
    }

    Ok(mem)
}

/// Generates a deterministic key for storage trie roots.
/// Combines contract address and block number into a u64 key.
/// Uses a different key space (high byte = 0x80+) to avoid collisions with class/contract tries.
fn storage_trie_root_key(address: ContractAddress, block: BlockNumber) -> u64 {
    let addr_bytes = Felt::from(address).to_bytes_be();
    let addr_hash = u64::from_be_bytes(addr_bytes[24..32].try_into().unwrap());
    let mixed = addr_hash.wrapping_mul(0x517cc1b727220a95) ^ block;
    0x80_00000000000000 | (mixed & 0x00FFFFFFFFFFFFFF)
}

/// Default capacity for the node cache (number of entries).
const NODE_CACHE_CAPACITY: usize = 4096;

/// DB-backed storage implementation for the trie.
///
/// Includes an LRU cache for node entries to avoid redundant DB reads.
/// Both `get()` and `hash()` read from the same underlying `TrieNodeEntry`,
/// so caching the full entry serves both lookups.
pub struct DbTrieStorage<Tb, Tx>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Tx: DbTx,
{
    tx: Tx,
    /// LRU cache for trie node entries, keyed by node index.
    node_cache: std::cell::RefCell<quick_cache::unsync::Cache<u64, TrieNodeEntry>>,
    _phantom: std::marker::PhantomData<Tb>,
}

impl<Tb, Tx> std::fmt::Debug for DbTrieStorage<Tb, Tx>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
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

impl<Tb, Tx> DbTrieStorage<Tb, Tx>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Tx: DbTx,
{
    pub fn new(tx: Tx) -> Self {
        Self {
            tx,
            node_cache: std::cell::RefCell::new(quick_cache::unsync::Cache::new(
                NODE_CACHE_CAPACITY,
            )),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Fetches a node entry, returning a cached copy if available.
    fn get_cached_entry(&self, index: TrieNodeIndex) -> anyhow::Result<Option<TrieNodeEntry>> {
        if let Some(entry) = self.node_cache.borrow_mut().get(&index.0) {
            return Ok(Some(entry.clone()));
        }

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

impl<Tb, Tx> katana_trie::Storage for DbTrieStorage<Tb, Tx>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
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
}

/// Persists a TrieUpdate to the database.
pub fn persist_trie_update<Tb, Tx>(
    tx: &Tx,
    update: &TrieUpdate,
    block: BlockNumber,
    trie_type: TrieType,
    next_index: &mut u64,
) -> anyhow::Result<Option<TrieNodeIndex>>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
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

    *next_index = base_index + update.nodes_added.len() as u64;
    let root_index = base_index + update.nodes_added.len() as u64 - 1;

    let root_key = storage_trie_root_key(address, block);
    tx.put::<tables::TrieRoots>(root_key, root_index)
        .map_err(|e| anyhow::anyhow!("DB error writing storage trie root: {e}"))?;

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
        Node::LeafBinary { left_hash, right_hash } => {
            StoredNode::LeafBinary { left_hash: *left_hash, right_hash: *right_hash }
        }
        Node::LeafEdge { path, child_hash } => {
            StoredNode::LeafEdge { path: path.clone(), child_hash: *child_hash }
        }
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
        revert_trie_block::<tables::TrieClassNodes, Tx>(
            tx,
            trie_composite_key(TrieType::Classes, block),
        )?;

        revert_trie_block::<tables::TrieContractNodes, Tx>(
            tx,
            trie_composite_key(TrieType::Contracts, block),
        )?;
    }

    Ok(())
}

fn revert_trie_block<Tb, Tx>(tx: &Tx, log_key: u64) -> anyhow::Result<()>
where
    Tb: tables::Table<Key = u64, Value = TrieNodeEntry>,
    Tx: DbTxMut,
{
    if let Some(added) = tx
        .get::<tables::TrieBlockLog>(log_key)
        .map_err(|e| anyhow::anyhow!("DB error reading block log: {e}"))?
    {
        for index in added.iter() {
            let _ = tx.delete::<Tb>(index, None);
        }

        let _ = tx.delete::<tables::TrieRoots>(log_key, None);
        let _ = tx.delete::<tables::TrieBlockLog>(log_key, None);
    }

    Ok(())
}

/// Prunes trie data for a block (removes roots and block log, but not nodes).
pub fn prune_trie_block<Tx: DbTxMut>(tx: &Tx, block: BlockNumber) -> anyhow::Result<()> {
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
    fn test_classes_trie_multi_block_with_db() {
        let db = test_utils::create_test_db();

        // Block 0: insert some classes
        {
            let tx = db.tx_mut().unwrap();
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            trie.insert(Felt::from(1u64), Felt::from(100u64)).unwrap();
            trie.insert(Felt::from(2u64), Felt::from(200u64)).unwrap();

            let update = trie.commit().unwrap();
            assert_ne!(update.root_commitment, Felt::ZERO);

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, _>(
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
            let mut trie = factory.classes_trie(0);

            trie.insert(Felt::from(3u64), Felt::from(300u64)).unwrap();

            let update = trie.commit().unwrap();
            assert_ne!(update.root_commitment, Felt::ZERO);

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, _>(
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

        // Block 0
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            for i in 0u64..10 {
                let class_hash = ClassHash::from(Felt::from(i * 7919 + 1000));
                let compiled_hash = CompiledClassHash::from(Felt::from(i * 100 + 100));
                trie.insert(class_hash, compiled_hash).unwrap();
            }

            let update = trie.commit().unwrap();

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

        // Block 1
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
            persist_trie_update::<tables::TrieClassNodes, _>(
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
            persist_trie_update::<tables::TrieClassNodes, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

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

        let mut rng_state = 0x12345678u64;
        let mut next_random_felt = || -> Felt {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let hi = rng_state;
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let lo = rng_state;
            let mut bytes = [0u8; 32];
            bytes[16..24].copy_from_slice(&hi.to_be_bytes());
            bytes[24..32].copy_from_slice(&lo.to_be_bytes());
            bytes[0] &= 0x07;
            Felt::from_bytes_be(&bytes)
        };

        // Block 0
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            for _ in 0u64..10 {
                trie.insert(next_random_felt(), next_random_felt()).unwrap();
            }

            let update = trie.commit().unwrap();

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();
        }

        // Block 1
        {
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            for _ in 0u64..10 {
                trie.insert(next_random_felt(), next_random_felt()).unwrap();
            }

            let update = trie.commit().unwrap();

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, _>(
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
    fn test_in_memory_trie_matches_db_trie() {
        // Verify that loading a trie into memory and committing produces the same root
        // as using the DB-backed trie directly.
        let db = test_utils::create_test_db();

        // Block 0: insert some classes via DB-backed trie
        let root0;
        {
            let tx = db.tx_mut().unwrap();
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);

            trie.insert(Felt::from(1u64), Felt::from(100u64)).unwrap();
            trie.insert(Felt::from(2u64), Felt::from(200u64)).unwrap();
            trie.insert(Felt::from(3u64), Felt::from(300u64)).unwrap();

            let update = trie.commit().unwrap();
            root0 = update.root_commitment;

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();

            tx.commit().unwrap();
        }

        // Block 1: insert more classes via in-memory trie loaded from DB
        let root1_in_memory;
        {
            let tx = db.tx_mut().unwrap();
            let factory = TrieDbFactory::new(tx.clone());

            let mut trie = factory.classes_trie_in_memory(0).unwrap();
            trie.insert(Felt::from(4u64), Felt::from(400u64)).unwrap();
            trie.insert(Felt::from(5u64), Felt::from(500u64)).unwrap();

            let update = trie.commit().unwrap();
            root1_in_memory = update.root_commitment;
            assert_ne!(root1_in_memory, Felt::ZERO);
            assert_ne!(root1_in_memory, root0);

            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, _>(
                &tx,
                &update,
                1,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();

            tx.commit().unwrap();
        }

        // Verify: do the same operation via DB-backed trie and compare roots
        let root1_db;
        {
            let db2 = test_utils::create_test_db();
            let tx = db2.tx_mut().unwrap();
            let factory = TrieDbFactory::new(tx.clone());

            // Recreate block 0
            let mut trie = factory.classes_trie(0);
            trie.insert(Felt::from(1u64), Felt::from(100u64)).unwrap();
            trie.insert(Felt::from(2u64), Felt::from(200u64)).unwrap();
            trie.insert(Felt::from(3u64), Felt::from(300u64)).unwrap();

            let update = trie.commit().unwrap();
            let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
            persist_trie_update::<tables::TrieClassNodes, _>(
                &tx,
                &update,
                0,
                TrieType::Classes,
                &mut next_idx,
            )
            .unwrap();

            // Block 1 via DB-backed trie
            let factory = TrieDbFactory::new(tx.clone());
            let mut trie = factory.classes_trie(0);
            trie.insert(Felt::from(4u64), Felt::from(400u64)).unwrap();
            trie.insert(Felt::from(5u64), Felt::from(500u64)).unwrap();

            let update = trie.commit().unwrap();
            root1_db = update.root_commitment;

            tx.commit().unwrap();
        }

        assert_eq!(
            root1_in_memory, root1_db,
            "In-memory and DB-backed trie should produce identical roots"
        );
    }

    #[test]
    fn test_load_trie_to_memory() {
        // Verify that load_trie_to_memory correctly loads all nodes
        let db = test_utils::create_test_db();
        let tx = db.tx_mut().unwrap();

        let factory = TrieDbFactory::new(tx.clone());
        let mut trie = factory.classes_trie(0);

        trie.insert(Felt::from(1u64), Felt::from(100u64)).unwrap();
        trie.insert(Felt::from(2u64), Felt::from(200u64)).unwrap();

        let update = trie.commit().unwrap();
        let original_root = update.root_commitment;

        let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&tx).unwrap();
        persist_trie_update::<tables::TrieClassNodes, _>(
            &tx,
            &update,
            0,
            TrieType::Classes,
            &mut next_idx,
        )
        .unwrap();

        // Load into memory and verify root hash matches
        let factory = TrieDbFactory::new(tx.clone());
        let mem_trie = factory.classes_trie_in_memory(0).unwrap();
        let root = mem_trie.root_hash().unwrap();
        assert_eq!(root, original_root);

        tx.commit().unwrap();
    }
}
