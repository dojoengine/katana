use core::fmt;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;

use anyhow::Result;
use katana_primitives::block::BlockNumber;
use katana_primitives::ContractAddress;
use katana_trie::bonsai::{BonsaiDatabase, BonsaiPersistentDatabase, ByteVec, DatabaseKey};
use katana_trie::CommitId;
use smallvec::ToSmallVec;

use crate::abstraction::{DbCursor, DbDupSortCursor, DbTx, DbTxMut};
use crate::models::trie::{TrieDatabaseKey, TrieDatabaseKeyType, TrieHistoryEntry};
use crate::models::{self};
use crate::tables::{self, Trie};

mod snapshot;

pub use snapshot::SnapshotTrieDb;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error(#[from] crate::error::DatabaseError);

impl katana_trie::bonsai::DBError for Error {}

impl Error {
    /// Returns the inner database error.
    pub fn into_inner(self) -> crate::error::DatabaseError {
        self.0
    }
}

#[derive(Debug)]
pub struct TrieDbFactory<Tx: DbTx> {
    tx: Tx,
}

impl<Tx: DbTx> TrieDbFactory<Tx> {
    pub fn new(tx: Tx) -> Self {
        Self { tx }
    }

    pub fn latest(&self) -> GlobalTrie<Tx> {
        GlobalTrie { tx: self.tx.clone() }
    }

    // TODO: check that the snapshot for the block number is available
    pub fn historical(&self, block: BlockNumber) -> Option<HistoricalGlobalTrie<Tx>> {
        Some(HistoricalGlobalTrie { tx: self.tx.clone(), block })
    }
}

/// Provides access to the latest tries.
#[derive(Debug)]
pub struct GlobalTrie<Tx: DbTx> {
    tx: Tx,
}

impl<Tx: DbTx> GlobalTrie<Tx> {
    /// Returns the contracts trie.
    pub fn contracts_trie(&self) -> katana_trie::ContractsTrie<TrieDb<tables::ContractsTrie, Tx>> {
        katana_trie::ContractsTrie::new(TrieDb::new(self.tx.clone()))
    }

    pub fn partial_contracts_trie(
        &self,
    ) -> katana_trie::PartialContractsTrie<TrieDb<tables::ContractsTrie, Tx>> {
        katana_trie::PartialContractsTrie::new_partial(TrieDb::new(self.tx.clone()))
    }

    /// Returns the classes trie.
    pub fn classes_trie(&self) -> katana_trie::ClassesTrie<TrieDb<tables::ClassesTrie, Tx>> {
        katana_trie::ClassesTrie::new(TrieDb::new(self.tx.clone()))
    }

    pub fn partial_classes_trie(
        &self,
    ) -> katana_trie::PartialClassesTrie<TrieDb<tables::ClassesTrie, Tx>> {
        katana_trie::PartialClassesTrie::new_partial(TrieDb::new(self.tx.clone()))
    }

    // TODO: makes this return an Option
    /// Returns the storages trie.
    pub fn storages_trie(
        &self,
        address: ContractAddress,
    ) -> katana_trie::StoragesTrie<TrieDb<tables::StoragesTrie, Tx>> {
        katana_trie::StoragesTrie::new(TrieDb::new(self.tx.clone()), address)
    }

    pub fn partial_storages_trie(
        &self,
        address: ContractAddress,
    ) -> katana_trie::PartialStoragesTrie<TrieDb<tables::StoragesTrie, Tx>> {
        katana_trie::PartialStoragesTrie::new_partial(TrieDb::new(self.tx.clone()), address)
    }
}

/// Historical tries, allowing access to the state tries at each block.
#[derive(Debug)]
pub struct HistoricalGlobalTrie<Tx: DbTx> {
    /// The database transaction.
    tx: Tx,
    /// The block number at which the trie was constructed.
    block: BlockNumber,
}

impl<Tx: DbTx> HistoricalGlobalTrie<Tx> {
    /// Returns the historical contracts trie.
    pub fn contracts_trie(
        &self,
    ) -> katana_trie::ContractsTrie<SnapshotTrieDb<tables::ContractsTrie, Tx>> {
        let commit = CommitId::new(self.block);
        katana_trie::ContractsTrie::new(SnapshotTrieDb::new(self.tx.clone(), commit))
    }

    pub fn partial_contracts_trie(
        &self,
    ) -> katana_trie::PartialContractsTrie<SnapshotTrieDb<tables::ContractsTrie, Tx>> {
        let commit = CommitId::new(self.block);
        katana_trie::PartialContractsTrie::new_partial(SnapshotTrieDb::new(self.tx.clone(), commit))
    }

    /// Returns the historical classes trie.
    pub fn classes_trie(
        &self,
    ) -> katana_trie::ClassesTrie<SnapshotTrieDb<tables::ClassesTrie, Tx>> {
        let commit = CommitId::new(self.block);
        katana_trie::ClassesTrie::new(SnapshotTrieDb::new(self.tx.clone(), commit))
    }

    pub fn partial_classes_trie(
        &self,
    ) -> katana_trie::PartialClassesTrie<SnapshotTrieDb<tables::ClassesTrie, Tx>> {
        let commit = CommitId::new(self.block);
        katana_trie::PartialClassesTrie::new_partial(SnapshotTrieDb::new(self.tx.clone(), commit))
    }

    // TODO: makes this return an Option
    /// Returns the historical storages trie.
    pub fn storages_trie(
        &self,
        address: ContractAddress,
    ) -> katana_trie::StoragesTrie<SnapshotTrieDb<tables::StoragesTrie, Tx>> {
        let commit = CommitId::new(self.block);
        katana_trie::StoragesTrie::new(SnapshotTrieDb::new(self.tx.clone(), commit), address)
    }

    pub fn partial_storages_trie(
        &self,
        address: ContractAddress,
    ) -> katana_trie::PartialStoragesTrie<SnapshotTrieDb<tables::StoragesTrie, Tx>> {
        let commit = CommitId::new(self.block);
        katana_trie::PartialStoragesTrie::new_partial(
            SnapshotTrieDb::new(self.tx.clone(), commit),
            address,
        )
    }
}

// --- Trie's database implementations. These are implemented based on the Bonsai Trie
// functionalities and abstractions.

pub struct TrieDb<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTx,
{
    tx: Tx,
    _phantom: PhantomData<Tb>,
}

impl<Tb, Tx> fmt::Debug for TrieDb<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTx,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrieDbMut").field("tx", &"..").finish()
    }
}

impl<Tb, Tx> TrieDb<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTx,
{
    pub(crate) fn new(tx: Tx) -> Self {
        Self { tx, _phantom: PhantomData }
    }
}

impl<Tb, Tx> BonsaiDatabase for TrieDb<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTx,
{
    type Batch = ();
    type DatabaseError = Error;

    fn create_batch(&self) -> Self::Batch {}

    fn remove_by_prefix(&mut self, _: &DatabaseKey<'_>) -> Result<(), Self::DatabaseError> {
        Ok(())
    }

    fn get(&self, key: &DatabaseKey<'_>) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let value = self.tx.get::<Tb>(to_db_key(key))?;
        Ok(value)
    }

    fn get_by_prefix(
        &self,
        prefix: &DatabaseKey<'_>,
    ) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        let mut results = Vec::new();

        let mut cursor = self.tx.cursor::<Tb>()?;
        let walker = cursor.walk(None)?;

        for entry in walker {
            let (TrieDatabaseKey { key, .. }, value) = entry?;

            if key.starts_with(prefix.as_slice()) {
                results.push((key.to_smallvec(), value));
            }
        }

        Ok(results)
    }

    fn insert(
        &mut self,
        _: &DatabaseKey<'_>,
        _: &[u8],
        _: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        unimplemented!("not supported in read-only transaction")
    }

    fn remove(
        &mut self,
        _: &DatabaseKey<'_>,
        _: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        unimplemented!("not supported in read-only transaction")
    }

    fn contains(&self, key: &DatabaseKey<'_>) -> Result<bool, Self::DatabaseError> {
        let key = to_db_key(key);
        let value = self.tx.get::<Tb>(key)?;
        Ok(value.is_some())
    }

    fn write_batch(&mut self, _: Self::Batch) -> Result<(), Self::DatabaseError> {
        unimplemented!("not supported in read-only transaction")
    }
}

pub struct TrieDbMut<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTxMut,
{
    tx: Tx,
    /// List of key-value pairs that has been added throughout the duration of the trie
    /// transaction.
    ///
    /// This will be used to create the trie snapshot.
    write_cache: HashMap<TrieDatabaseKey, ByteVec>,
    _phantom: PhantomData<Tb>,
}

impl<Tb, Tx> fmt::Debug for TrieDbMut<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTxMut,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrieDbMut").field("tx", &"..").finish()
    }
}

impl<Tb, Tx> TrieDbMut<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTxMut,
{
    pub fn new(tx: Tx) -> Self {
        Self { tx, write_cache: HashMap::new(), _phantom: PhantomData }
    }

    /// Removes the snapshot data for the given block number.
    ///
    /// This is the inverse of [`BonsaiPersistentDatabase::snapshot`] - it removes all history
    /// entries for the given block and updates the corresponding changesets.
    ///
    /// Note: There is currently no efficient way to check if a snapshot exists for a given block
    /// without querying the `Tb::History` table. As a result, calling this method on a
    /// non-existent snapshot is a no-op.
    pub fn remove_snapshot(&mut self, block: BlockNumber) -> Result<(), Error> {
        // Get all history entries for this block using dupsort cursor
        let mut cursor = self.tx.cursor_dup::<Tb::History>()?;

        // walk_dup iterates only over entries with the same key (block number)
        let Some(walker) = cursor.walk_dup(Some(block), None)? else {
            // No entries for this block
            return Ok(());
        };

        let mut keys_to_update = Vec::new();
        for entry in walker {
            let (_, entry) = entry?;
            keys_to_update.push(entry.key);
        }

        // For each key, update its changeset by removing this block number
        for key in &keys_to_update {
            if let Some(mut set) = self.tx.get::<Tb::Changeset>(key.clone())? {
                set.remove(block);
                if set.is_empty() {
                    self.tx.delete::<Tb::Changeset>(key.clone(), None)?;
                } else {
                    self.tx.put::<Tb::Changeset>(key.clone(), set)?;
                }
            }
        }

        // Delete all history entries for this block using dupsort cursor
        let mut cursor = self.tx.cursor_dup_mut::<Tb::History>()?;

        let Some(mut walker) = cursor.walk_dup(Some(block), None)? else {
            return Ok(());
        };

        // Use delete_current to delete each entry as we iterate
        while walker.next().is_some() {
            walker.delete_current()?;
        }

        Ok(())
    }
}

impl<Tb, Tx> BonsaiDatabase for TrieDbMut<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTxMut,
{
    type Batch = ();
    type DatabaseError = Error;

    fn create_batch(&self) -> Self::Batch {}

    fn remove_by_prefix(&mut self, prefix: &DatabaseKey<'_>) -> Result<(), Self::DatabaseError> {
        let mut cursor = self.tx.cursor_mut::<Tb>()?;
        let walker = cursor.walk(None)?;

        let mut keys_to_remove = Vec::new();
        // iterate over all entries in the table
        for entry in walker {
            let (key, _) = entry?;

            match key.r#type {
                TrieDatabaseKeyType::Flat => {
                    if let DatabaseKey::Flat(prefix_key) = prefix {
                        if key.key.starts_with(prefix_key) {
                            keys_to_remove.push(key);
                        }
                    }
                }
                TrieDatabaseKeyType::Trie => {
                    if let DatabaseKey::Trie(prefix_key) = prefix {
                        if key.key.starts_with(prefix_key) {
                            keys_to_remove.push(key);
                        }
                    }
                }
                TrieDatabaseKeyType::TrieLog => {
                    if let DatabaseKey::TrieLog(prefix_key) = prefix {
                        if key.key.starts_with(prefix_key) {
                            keys_to_remove.push(key);
                        }
                    }
                }
            }
        }

        for key in keys_to_remove {
            let _ = self.tx.delete::<Tb>(key, None)?;
        }

        Ok(())
    }

    fn get(&self, key: &DatabaseKey<'_>) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let value = self.tx.get::<Tb>(to_db_key(key))?;
        Ok(value)
    }

    fn get_by_prefix(
        &self,
        prefix: &DatabaseKey<'_>,
    ) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        TrieDb::<Tb, Tx>::new(self.tx.clone()).get_by_prefix(prefix)
    }

    fn insert(
        &mut self,
        key: &DatabaseKey<'_>,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let _ = batch;
        let key = to_db_key(key);
        let value: ByteVec = value.to_smallvec();

        let old_value = self.tx.get::<Tb>(key.clone())?;
        self.tx.put::<Tb>(key.clone(), value.clone())?;

        self.write_cache.insert(key, value);
        Ok(old_value)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey<'_>,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let _ = batch;
        let key = to_db_key(key);

        let old_value = self.tx.get::<Tb>(key.clone())?;
        self.tx.delete::<Tb>(key, None)?;

        Ok(old_value)
    }

    fn contains(&self, key: &DatabaseKey<'_>) -> Result<bool, Self::DatabaseError> {
        let key = to_db_key(key);
        let value = self.tx.get::<Tb>(key)?;
        Ok(value.is_some())
    }

    fn write_batch(&mut self, _: Self::Batch) -> Result<(), Self::DatabaseError> {
        Ok(())
    }
}

impl<Tb, Tx> BonsaiPersistentDatabase<CommitId> for TrieDbMut<Tb, Tx>
where
    Tb: Trie,
    Tx: DbTxMut,
{
    type DatabaseError = Error;
    type Transaction<'a>
        = SnapshotTrieDb<Tb, Tx>
    where
        Self: 'a;

    fn snapshot(&mut self, id: CommitId) {
        let block_number: BlockNumber = id.into();

        let entries = std::mem::take(&mut self.write_cache);
        let entries = entries.into_iter().map(|(key, value)| TrieHistoryEntry { key, value });

        for entry in entries {
            let mut set = self
                .tx
                .get::<Tb::Changeset>(entry.key.clone())
                .expect("failed to get trie change set")
                .unwrap_or_default();
            set.insert(block_number);

            self.tx
                .put::<Tb::Changeset>(entry.key.clone(), set)
                .expect("failed to put trie change set");

            self.tx
                .put::<Tb::History>(block_number, entry)
                .expect("failed to put trie history entry");
        }
    }

    // merging should recompute the trie again
    fn merge<'a>(&mut self, transaction: Self::Transaction<'a>) -> Result<(), Self::DatabaseError>
    where
        Self: 'a,
    {
        let _ = transaction;
        unimplemented!();
    }

    // TODO: check if the snapshot exist
    fn transaction(&self, id: CommitId) -> Option<(CommitId, Self::Transaction<'_>)> {
        Some((id, SnapshotTrieDb::new(self.tx.clone(), id)))
    }
}

fn to_db_key(key: &DatabaseKey<'_>) -> models::trie::TrieDatabaseKey {
    match key {
        DatabaseKey::Flat(bytes) => {
            TrieDatabaseKey { key: bytes.to_vec(), r#type: TrieDatabaseKeyType::Flat }
        }
        DatabaseKey::Trie(bytes) => {
            TrieDatabaseKey { key: bytes.to_vec(), r#type: TrieDatabaseKeyType::Trie }
        }
        DatabaseKey::TrieLog(bytes) => {
            TrieDatabaseKey { key: bytes.to_vec(), r#type: TrieDatabaseKeyType::TrieLog }
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::cairo::ShortString;
    use katana_primitives::hash::{Poseidon, StarkHash};
    use katana_primitives::{felt, hash};
    use katana_trie::{verify_proof, ClassesTrie, CommitId};

    use super::TrieDbMut;
    use crate::abstraction::Database;
    use crate::mdbx::test_utils;
    use crate::tables;
    use crate::trie::SnapshotTrieDb;

    #[test]
    fn snapshot() {
        let db = test_utils::create_test_db();
        let tx = db.tx_mut().expect("failed to get tx");

        let mut trie = ClassesTrie::new(TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone()));

        let root0 = {
            let entries = [
                (felt!("0x9999"), felt!("0xdead")),
                (felt!("0x5555"), felt!("0xbeef")),
                (felt!("0x1337"), felt!("0xdeadbeef")),
            ];

            for (key, value) in entries {
                trie.insert(key, value);
            }

            trie.commit(0);
            trie.root()
        };

        let root1 = {
            let entries = [
                (felt!("0x6969"), felt!("0x80085")),
                (felt!("0x3333"), felt!("0x420")),
                (felt!("0x2222"), felt!("0x7171")),
            ];

            for (key, value) in entries {
                trie.insert(key, value);
            }

            trie.commit(1);
            trie.root()
        };

        assert_ne!(root0, root1);

        {
            let db = SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), CommitId::new(0));
            let mut snapshot0 = ClassesTrie::new(db);

            let snapshot_root0 = snapshot0.root();
            assert_eq!(snapshot_root0, root0);

            let proofs0 = snapshot0.multiproof(vec![felt!("0x9999")]);
            let verify_result0 =
                verify_proof::<Poseidon>(&proofs0, snapshot_root0, vec![felt!("0x9999")]);

            let value = hash::Poseidon::hash(
                &ShortString::from_ascii("CONTRACT_CLASS_LEAF_V0").into(),
                &felt!("0xdead"),
            );
            assert_eq!(vec![value], verify_result0);
        }

        {
            let commit = CommitId::new(1);
            let mut snapshot1 =
                ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), commit));

            let snapshot_root1 = snapshot1.root();
            assert_eq!(snapshot_root1, root1);

            let proofs1 = snapshot1.multiproof(vec![felt!("0x6969")]);
            let verify_result1 =
                verify_proof::<Poseidon>(&proofs1, snapshot_root1, vec![felt!("0x6969")]);

            let value = hash::Poseidon::hash(
                &ShortString::from_ascii("CONTRACT_CLASS_LEAF_V0").into(),
                &felt!("0x80085"),
            );
            assert_eq!(vec![value], verify_result1);
        }

        {
            let root = trie.root();
            let proofs = trie.multiproof(vec![felt!("0x6969"), felt!("0x9999")]);
            let result =
                verify_proof::<Poseidon>(&proofs, root, vec![felt!("0x6969"), felt!("0x9999")]);

            let value0 = hash::Poseidon::hash(
                &ShortString::from_ascii("CONTRACT_CLASS_LEAF_V0").into(),
                &felt!("0x80085"),
            );
            let value1 = hash::Poseidon::hash(
                &ShortString::from_ascii("CONTRACT_CLASS_LEAF_V0").into(),
                &felt!("0xdead"),
            );

            assert_eq!(vec![value0, value1], result);
        }
    }

    #[test]
    fn revert_to() {
        let db = test_utils::create_test_db();
        let tx = db.tx_mut().expect("failed to get tx");

        let mut trie = ClassesTrie::new(TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone()));

        // Insert values at block 0
        trie.insert(felt!("0x1"), felt!("0x100"));
        trie.insert(felt!("0x2"), felt!("0x200"));
        trie.commit(0);
        let root_at_block_0 = trie.root();

        // Insert more values at block 1
        trie.insert(felt!("0x3"), felt!("0x300"));
        trie.insert(felt!("0x4"), felt!("0x400"));
        trie.commit(1);
        let root_at_block_1 = trie.root();

        // Roots should be different
        assert_ne!(root_at_block_0, root_at_block_1);

        // Insert even more values at block 2
        trie.insert(felt!("0x5"), felt!("0x500"));
        trie.commit(2);
        let root_at_block_2 = trie.root();

        // Roots should be different
        assert_ne!(root_at_block_1, root_at_block_2);
        assert_ne!(root_at_block_0, root_at_block_2);

        // Revert to block 1
        trie.revert_to(1, 2);
        let root_after_revert = trie.root();

        // After revert, root should match block 1
        assert_eq!(root_after_revert, root_at_block_1);

        // Revert to block 0
        trie.revert_to(0, 1);
        let root_after_second_revert = trie.root();

        // After revert, root should match block 0
        assert_eq!(root_after_second_revert, root_at_block_0);

        // Insert more values at block 1
        trie.insert(felt!("0x3"), felt!("0x300"));
        trie.insert(felt!("0x4"), felt!("0x400"));
        trie.commit(1);
        let root_at_block_1_after_insert = trie.root();

        // After insertion, root should match block 1
        assert_eq!(root_at_block_1_after_insert, root_at_block_1);

        // Insert even more values at block 2
        trie.insert(felt!("0x5"), felt!("0x500"));
        trie.commit(2);
        let root_at_block_2_after_insert = trie.root();

        // After insertion, root should match block 2
        assert_eq!(root_at_block_2_after_insert, root_at_block_2);
    }

    /// Tests the `remove_snapshot` method by creating multiple snapshots and removing them.
    ///
    /// Note: This test verifies that remaining snapshots still work correctly after removal,
    /// but does not explicitly verify that removed snapshots no longer exist. This is because
    /// there is currently no efficient way to check snapshot existence at the `SnapshotTrieDb`
    /// level without querying the underlying `Tb::History` table directly.
    #[test]
    fn remove_snapshot() {
        use katana_primitives::Felt;

        let db = test_utils::create_test_db();
        let tx = db.tx_mut().expect("failed to get tx");

        let mut trie = ClassesTrie::new(TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone()));

        ////////////////////////////////////////////////////////////////////////////////////
        // Setup: Create snapshots at blocks 0-4 with various insertions and updates
        ////////////////////////////////////////////////////////////////////////////////////

        // Block 0: Insert 50 new values
        for i in 0u64..50 {
            trie.insert(Felt::from(i), Felt::from(i * 100));
        }
        trie.commit(0);
        let root_at_block_0 = trie.root();

        // Block 1: Insert 50 new values + update 10 existing keys from block 0
        for i in 50u64..100 {
            trie.insert(Felt::from(i), Felt::from(i * 100));
        }
        for i in 10u64..20 {
            trie.insert(Felt::from(i), Felt::from(i * 200));
        }
        trie.commit(1);
        let root_at_block_1 = trie.root();
        assert_ne!(root_at_block_0, root_at_block_1);

        // Block 2: Insert 50 new values + update 10 existing keys from block 1
        for i in 100u64..150 {
            trie.insert(Felt::from(i), Felt::from(i * 100));
        }
        for i in 60u64..70 {
            trie.insert(Felt::from(i), Felt::from(i * 300));
        }
        trie.commit(2);
        let root_at_block_2 = trie.root();
        assert_ne!(root_at_block_1, root_at_block_2);

        // Block 3: Insert 50 new values
        for i in 150u64..200 {
            trie.insert(Felt::from(i), Felt::from(i * 100));
        }
        trie.commit(3);
        let root_at_block_3 = trie.root();
        assert_ne!(root_at_block_2, root_at_block_3);

        // Block 4: Insert 50 new values
        for i in 200u64..250 {
            trie.insert(Felt::from(i), Felt::from(i * 100));
        }
        trie.commit(4);
        let root_at_block_4 = trie.root();
        assert_ne!(root_at_block_3, root_at_block_4);

        ////////////////////////////////////////////////////////////////////////////////////
        // Verify: All snapshots (blocks 0-4) exist and have correct roots
        ////////////////////////////////////////////////////////////////////////////////////

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 0.into()));
        assert_eq!(snapshot.root(), root_at_block_0);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 1.into()));
        assert_eq!(snapshot.root(), root_at_block_1);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 2.into()));
        assert_eq!(snapshot.root(), root_at_block_2);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 3.into()));
        assert_eq!(snapshot.root(), root_at_block_3);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 4.into()));
        assert_eq!(snapshot.root(), root_at_block_4);

        ////////////////////////////////////////////////////////////////////////////////////
        // Remove snapshot at block 1
        ////////////////////////////////////////////////////////////////////////////////////

        let mut trie_db = TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone());
        trie_db.remove_snapshot(1).expect("failed to remove snapshot");

        // snapshots at blocks 0, 2, 3, 4 should still exist

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 0.into()));
        assert_eq!(snapshot.root(), root_at_block_0);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 2.into()));
        assert_eq!(snapshot.root(), root_at_block_2);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 3.into()));
        assert_eq!(snapshot.root(), root_at_block_3);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 4.into()));
        assert_eq!(snapshot.root(), root_at_block_4);

        ////////////////////////////////////////////////////////////////////////////////////
        // Remove snapshots at blocks 0 and 2
        ////////////////////////////////////////////////////////////////////////////////////

        let mut trie_db = TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone());
        trie_db.remove_snapshot(0).expect("failed to remove snapshot");
        trie_db.remove_snapshot(2).expect("failed to remove snapshot");

        // snapshots at blocks 3 and 4 should still exist

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 3.into()));
        assert_eq!(snapshot.root(), root_at_block_3);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 4.into()));
        assert_eq!(snapshot.root(), root_at_block_4);

        ////////////////////////////////////////////////////////////////////////////////////
        // Remove snapshot at block 3
        ////////////////////////////////////////////////////////////////////////////////////

        let mut trie_db = TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone());
        trie_db.remove_snapshot(3).expect("failed to remove snapshot");

        // snapshot at block 4 should still exist

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 4.into()));
        assert_eq!(snapshot.root(), root_at_block_4);

        ////////////////////////////////////////////////////////////////////////////////////
        // Verify: Trie still works after pruning - insert new values at block 5
        ////////////////////////////////////////////////////////////////////////////////////

        let mut trie = ClassesTrie::new(TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone()));
        for i in 250u64..300 {
            trie.insert(Felt::from(i), Felt::from(i * 100));
        }
        trie.commit(5);
        let root_at_block_5 = trie.root();
        assert_ne!(root_at_block_4, root_at_block_5);

        // both remaining snapshots (blocks 4 and 5) should exist

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 4.into()));
        assert_eq!(snapshot.root(), root_at_block_4);

        let snapshot =
            ClassesTrie::new(SnapshotTrieDb::<tables::ClassesTrie, _>::new(tx.clone(), 5.into()));
        assert_eq!(snapshot.root(), root_at_block_5);
    }
}
