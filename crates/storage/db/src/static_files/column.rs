use std::io;

use super::store::StaticStore;

/// Index entry: 8 bytes for offset + 4 bytes for length = 12 bytes.
const INDEX_ENTRY_SIZE: usize = 12;

/// A column of fixed-size records, addressed by sequential u64 key.
///
/// Direct offset calculation: `offset = key * record_size`, so no index file is needed.
pub struct FixedColumn<S: StaticStore> {
    store: S,
    record_size: usize,
}

impl<S: StaticStore> FixedColumn<S> {
    pub fn new(store: S, record_size: usize) -> Self {
        assert!(record_size > 0, "record_size must be positive");
        Self { store, record_size }
    }

    /// Get a record by sequential key. Returns `None` if the key is beyond the current count.
    pub fn get(&self, key: u64) -> io::Result<Option<Vec<u8>>> {
        let offset = key * self.record_size as u64;
        let file_len = self.store.len()?;

        if offset + self.record_size as u64 > file_len {
            return Ok(None);
        }

        let data = self.store.read_at(offset, self.record_size)?;
        Ok(Some(data))
    }

    /// Append a record. The key must equal the current count (i.e., append-only).
    pub fn append(&self, key: u64, data: &[u8]) -> io::Result<()> {
        debug_assert_eq!(data.len(), self.record_size, "data length must match record_size");
        let expected_offset = key * self.record_size as u64;
        let actual_offset = self.store.append(data)?;
        debug_assert_eq!(
            expected_offset, actual_offset,
            "FixedColumn: key {key} does not match append offset"
        );
        Ok(())
    }

    /// Return the number of records currently stored.
    pub fn count(&self) -> io::Result<u64> {
        let len = self.store.len()?;
        Ok(len / self.record_size as u64)
    }

    pub fn sync(&self) -> io::Result<()> {
        self.store.sync()
    }

    /// Truncate to exactly `count` records.
    pub fn truncate_to(&self, count: u64) -> io::Result<()> {
        let new_len = count * self.record_size as u64;
        self.store.truncate(new_len)
    }
}

/// A column of variable-size records with an index for offset/length lookup.
///
/// The data file (`.dat`) stores compressed values appended sequentially.
/// The index file (`.idx`) stores an array of `(offset: u64, length: u32)` = 12 bytes per entry.
pub struct IndexedColumn<S: StaticStore> {
    data: S,
    index: S,
}

impl<S: StaticStore> IndexedColumn<S> {
    pub fn new(data: S, index: S) -> Self {
        Self { data, index }
    }

    /// Get a record by sequential key. Returns `None` if the key is beyond the current count.
    pub fn get(&self, key: u64) -> io::Result<Option<Vec<u8>>> {
        let idx_offset = key * INDEX_ENTRY_SIZE as u64;
        let idx_len = self.index.len()?;

        if idx_offset + INDEX_ENTRY_SIZE as u64 > idx_len {
            return Ok(None);
        }

        let idx_entry = self.index.read_at(idx_offset, INDEX_ENTRY_SIZE)?;
        let data_offset = u64::from_le_bytes(idx_entry[0..8].try_into().unwrap());
        let data_length = u32::from_le_bytes(idx_entry[8..12].try_into().unwrap()) as usize;

        if data_length == 0 {
            return Ok(Some(Vec::new()));
        }

        let data = self.data.read_at(data_offset, data_length)?;
        Ok(Some(data))
    }

    /// Append a record. The key must equal the current count (i.e., append-only).
    pub fn append(&self, key: u64, data: &[u8]) -> io::Result<()> {
        let data_offset = self.data.append(data)?;
        let data_length = data.len() as u32;

        let mut idx_entry = [0u8; INDEX_ENTRY_SIZE];
        idx_entry[0..8].copy_from_slice(&data_offset.to_le_bytes());
        idx_entry[8..12].copy_from_slice(&data_length.to_le_bytes());

        let expected_idx_offset = key * INDEX_ENTRY_SIZE as u64;
        let actual_idx_offset = self.index.append(&idx_entry)?;
        debug_assert_eq!(
            expected_idx_offset, actual_idx_offset,
            "IndexedColumn: key {key} does not match index append offset"
        );

        Ok(())
    }

    /// Return the number of records currently stored.
    pub fn count(&self) -> io::Result<u64> {
        let idx_len = self.index.len()?;
        Ok(idx_len / INDEX_ENTRY_SIZE as u64)
    }

    pub fn sync(&self) -> io::Result<()> {
        self.data.sync()?;
        self.index.sync()
    }

    /// Truncate to exactly `count` records.
    ///
    /// The index is truncated to `count * 12` bytes. The data file is truncated to the
    /// offset pointed to by the last remaining index entry (or 0 if count == 0).
    pub fn truncate_to(&self, count: u64) -> io::Result<()> {
        if count == 0 {
            self.data.truncate(0)?;
            self.index.truncate(0)?;
            return Ok(());
        }

        // Read the last valid index entry to find the data truncation point.
        let last_idx_offset = (count - 1) * INDEX_ENTRY_SIZE as u64;
        let idx_entry = self.index.read_at(last_idx_offset, INDEX_ENTRY_SIZE)?;
        let data_offset = u64::from_le_bytes(idx_entry[0..8].try_into().unwrap());
        let data_length = u32::from_le_bytes(idx_entry[8..12].try_into().unwrap()) as u64;

        self.data.truncate(data_offset + data_length)?;
        self.index.truncate(count * INDEX_ENTRY_SIZE as u64)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::static_files::store::MemoryStore;

    #[test]
    fn fixed_column_basic() {
        let store = MemoryStore::new();
        let col = FixedColumn::new(store, 32);

        assert_eq!(col.count().unwrap(), 0);
        assert!(col.get(0).unwrap().is_none());

        let data = [0xABu8; 32];
        col.append(0, &data).unwrap();
        assert_eq!(col.count().unwrap(), 1);

        let retrieved = col.get(0).unwrap().unwrap();
        assert_eq!(retrieved, data);

        assert!(col.get(1).unwrap().is_none());

        let data2 = [0xCDu8; 32];
        col.append(1, &data2).unwrap();
        assert_eq!(col.count().unwrap(), 2);

        let retrieved2 = col.get(1).unwrap().unwrap();
        assert_eq!(retrieved2, data2);

        col.truncate_to(1).unwrap();
        assert_eq!(col.count().unwrap(), 1);
        assert!(col.get(1).unwrap().is_none());
        assert_eq!(col.get(0).unwrap().unwrap(), data);
    }

    #[test]
    fn indexed_column_basic() {
        let data_store = MemoryStore::new();
        let idx_store = MemoryStore::new();
        let col = IndexedColumn::new(data_store, idx_store);

        assert_eq!(col.count().unwrap(), 0);
        assert!(col.get(0).unwrap().is_none());

        let record1 = b"hello world";
        col.append(0, record1).unwrap();
        assert_eq!(col.count().unwrap(), 1);

        let retrieved = col.get(0).unwrap().unwrap();
        assert_eq!(retrieved, record1);

        let record2 = b"a much longer record with variable length data";
        col.append(1, record2).unwrap();
        assert_eq!(col.count().unwrap(), 2);

        let retrieved2 = col.get(1).unwrap().unwrap();
        assert_eq!(retrieved2, record2);

        // Truncate back to 1 record.
        col.truncate_to(1).unwrap();
        assert_eq!(col.count().unwrap(), 1);
        assert!(col.get(1).unwrap().is_none());
        assert_eq!(col.get(0).unwrap().unwrap(), record1);
    }

    #[test]
    fn indexed_column_empty_record() {
        let col = IndexedColumn::new(MemoryStore::new(), MemoryStore::new());

        col.append(0, b"").unwrap();
        assert_eq!(col.count().unwrap(), 1);

        let retrieved = col.get(0).unwrap().unwrap();
        assert!(retrieved.is_empty());
    }

    #[test]
    fn fixed_column_8_byte_records() {
        let col = FixedColumn::new(MemoryStore::new(), 8);

        for i in 0u64..10 {
            col.append(i, &i.to_le_bytes()).unwrap();
        }

        assert_eq!(col.count().unwrap(), 10);

        for i in 0u64..10 {
            let data = col.get(i).unwrap().unwrap();
            let val = u64::from_le_bytes(data.try_into().unwrap());
            assert_eq!(val, i);
        }
    }

    #[test]
    fn truncate_to_zero() {
        let col = IndexedColumn::new(MemoryStore::new(), MemoryStore::new());

        col.append(0, b"data").unwrap();
        col.append(1, b"more data").unwrap();
        assert_eq!(col.count().unwrap(), 2);

        col.truncate_to(0).unwrap();
        assert_eq!(col.count().unwrap(), 0);
        assert!(col.get(0).unwrap().is_none());
    }
}
