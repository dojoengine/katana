use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use parking_lot::{Mutex, RwLock};

/// Low-level byte storage backend, generic over file/memory/etc.
pub trait StaticStore: Send + Sync + 'static {
    /// Read bytes at the given byte offset and length.
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>>;
    /// Append bytes to the end. Returns the offset where data was written.
    fn append(&self, data: &[u8]) -> io::Result<u64>;
    /// Current length in bytes.
    fn len(&self) -> io::Result<u64>;
    /// Flush any buffered data to durable storage.
    fn sync(&self) -> io::Result<()>;
    /// Truncate to the given length (for crash recovery).
    fn truncate(&self, len: u64) -> io::Result<()>;
}

/// File-backed store (production).
pub struct FileStore {
    file: Mutex<File>,
}

impl FileStore {
    /// Open or create a file at the given path.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file =
            OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        Ok(Self { file: Mutex::new(file) })
    }
}

impl StaticStore for FileStore {
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut file = self.file.lock();
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn append(&self, data: &[u8]) -> io::Result<u64> {
        let mut file = self.file.lock();
        let offset = file.seek(SeekFrom::End(0))?;
        file.write_all(data)?;
        Ok(offset)
    }

    fn len(&self) -> io::Result<u64> {
        let mut file = self.file.lock();
        file.seek(SeekFrom::End(0))
    }

    fn sync(&self) -> io::Result<()> {
        let file = self.file.lock();
        file.sync_all()
    }

    fn truncate(&self, len: u64) -> io::Result<()> {
        let file = self.file.lock();
        file.set_len(len)
    }
}

/// Memory-backed store (tests, in-memory mode).
pub struct MemoryStore {
    buf: RwLock<Vec<u8>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self { buf: RwLock::new(Vec::new()) }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl StaticStore for MemoryStore {
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let buf = self.buf.read();
        let start = offset as usize;
        let end = start + len;
        if end > buf.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read past end of buffer"));
        }
        Ok(buf[start..end].to_vec())
    }

    fn append(&self, data: &[u8]) -> io::Result<u64> {
        let mut buf = self.buf.write();
        let offset = buf.len() as u64;
        buf.extend_from_slice(data);
        Ok(offset)
    }

    fn len(&self) -> io::Result<u64> {
        let buf = self.buf.read();
        Ok(buf.len() as u64)
    }

    fn sync(&self) -> io::Result<()> {
        Ok(())
    }

    fn truncate(&self, len: u64) -> io::Result<()> {
        let mut buf = self.buf.write();
        buf.truncate(len as usize);
        Ok(())
    }
}

/// Type-erased store that can be either file-backed or memory-backed.
pub enum AnyStore {
    File(FileStore),
    Memory(MemoryStore),
}

impl StaticStore for AnyStore {
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        match self {
            AnyStore::File(s) => s.read_at(offset, len),
            AnyStore::Memory(s) => s.read_at(offset, len),
        }
    }

    fn append(&self, data: &[u8]) -> io::Result<u64> {
        match self {
            AnyStore::File(s) => s.append(data),
            AnyStore::Memory(s) => s.append(data),
        }
    }

    fn len(&self) -> io::Result<u64> {
        match self {
            AnyStore::File(s) => s.len(),
            AnyStore::Memory(s) => s.len(),
        }
    }

    fn sync(&self) -> io::Result<()> {
        match self {
            AnyStore::File(s) => s.sync(),
            AnyStore::Memory(s) => s.sync(),
        }
    }

    fn truncate(&self, len: u64) -> io::Result<()> {
        match self {
            AnyStore::File(s) => s.truncate(len),
            AnyStore::Memory(s) => s.truncate(len),
        }
    }
}

impl std::fmt::Debug for AnyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnyStore::File(_) => write!(f, "AnyStore::File"),
            AnyStore::Memory(_) => write!(f, "AnyStore::Memory"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let store = MemoryStore::new();
        assert_eq!(store.len().unwrap(), 0);

        let offset = store.append(b"hello").unwrap();
        assert_eq!(offset, 0);
        assert_eq!(store.len().unwrap(), 5);

        let offset2 = store.append(b" world").unwrap();
        assert_eq!(offset2, 5);
        assert_eq!(store.len().unwrap(), 11);

        let data = store.read_at(0, 5).unwrap();
        assert_eq!(&data, b"hello");

        let data = store.read_at(5, 6).unwrap();
        assert_eq!(&data, b" world");

        store.truncate(5).unwrap();
        assert_eq!(store.len().unwrap(), 5);

        let data = store.read_at(0, 5).unwrap();
        assert_eq!(&data, b"hello");
    }

    #[test]
    fn file_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.dat");

        let store = FileStore::open(&path).unwrap();
        assert_eq!(store.len().unwrap(), 0);

        let offset = store.append(b"hello").unwrap();
        assert_eq!(offset, 0);

        let offset2 = store.append(b" world").unwrap();
        assert_eq!(offset2, 5);

        let data = store.read_at(0, 5).unwrap();
        assert_eq!(&data, b"hello");

        let data = store.read_at(5, 6).unwrap();
        assert_eq!(&data, b" world");

        store.truncate(5).unwrap();
        assert_eq!(store.len().unwrap(), 5);
    }
}
