use std::io;

use super::store::StaticStore;

/// A column of fixed-size records, addressed by sequential u64 key.
///
/// Direct offset calculation: `offset = key * record_size`, so no external index is needed.
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

/// A column of variable-size records stored sequentially in a `.dat` file.
///
/// Unlike the previous `IndexedColumn`, this does NOT maintain an `.idx` file.
/// The caller is responsible for storing and providing the `(offset, length)` pointers
/// externally (in MDBX) to read data back.
pub struct DataColumn<S: StaticStore> {
    store: S,
}

impl<S: StaticStore> DataColumn<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Read data at the given byte offset and length.
    pub fn read(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        if length == 0 {
            return Ok(Vec::new());
        }
        self.store.read_at(offset, length as usize)
    }

    /// Append data to the end. Returns `(offset, length)` where the data was written.
    /// The caller must store these values externally (e.g., in MDBX) to read the data back.
    pub fn append(&self, data: &[u8]) -> io::Result<(u64, u32)> {
        let offset = self.store.append(data)?;
        Ok((offset, data.len() as u32))
    }

    /// Current file length in bytes.
    pub fn len(&self) -> io::Result<u64> {
        self.store.len()
    }

    pub fn sync(&self) -> io::Result<()> {
        self.store.sync()
    }

    /// Truncate the data file to the given byte length.
    pub fn truncate(&self, byte_len: u64) -> io::Result<()> {
        self.store.truncate(byte_len)
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
    fn data_column_basic() {
        let col = DataColumn::new(MemoryStore::new());

        let record1 = b"hello world";
        let (off1, len1) = col.append(record1).unwrap();
        assert_eq!(off1, 0);
        assert_eq!(len1, 11);

        let record2 = b"a much longer record with variable length data";
        let (off2, len2) = col.append(record2).unwrap();
        assert_eq!(off2, 11);

        let retrieved1 = col.read(off1, len1).unwrap();
        assert_eq!(retrieved1, record1);

        let retrieved2 = col.read(off2, len2).unwrap();
        assert_eq!(retrieved2, record2);
    }

    #[test]
    fn data_column_empty_record() {
        let col = DataColumn::new(MemoryStore::new());

        let (off, len) = col.append(b"").unwrap();
        assert_eq!(off, 0);
        assert_eq!(len, 0);

        let retrieved = col.read(off, len).unwrap();
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
}
