use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// Refresh any internal read caches (e.g., mmap) to cover newly-written data.
    /// No-op by default.
    fn remap(&self) -> io::Result<()> {
        Ok(())
    }
}

/// File-backed store using `pread`/`pwrite` for concurrent I/O, with mmap for reads.
///
/// - Reads use mmap when the data is within the mapped region, falling back to pread for data
///   written after the last remap.
/// - Writes use `pwrite` under a mutex for serialized appends.
/// - The mmap is refreshed on `remap()` to cover newly-written data.
pub struct FileStore {
    file: File,
    path: std::path::PathBuf,
    /// Serializes append writes.
    write_lock: Mutex<()>,
    /// Cached file length.
    cached_len: AtomicU64,
    /// Memory-mapped read view. RwLock allows concurrent reads, exclusive remap.
    mmap: RwLock<Option<memmap2::Mmap>>,
    /// Length covered by the current mmap.
    mmap_len: AtomicU64,
}

impl FileStore {
    /// Open or create a file at the given path.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file =
            OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        let len = file.metadata()?.len();

        let (mmap, mmap_len) = if len > 0 {
            let m = unsafe { memmap2::Mmap::map(&file)? };
            let ml = m.len() as u64;
            (Some(m), ml)
        } else {
            (None, 0)
        };

        Ok(Self {
            file,
            path: path.to_path_buf(),
            write_lock: Mutex::new(()),
            cached_len: AtomicU64::new(len),
            mmap: RwLock::new(mmap),
            mmap_len: AtomicU64::new(mmap_len),
        })
    }

    /// Refresh the mmap to cover all data written so far.
    /// Call this after a batch of appends to make new data visible via mmap reads.
    pub fn remap(&self) -> io::Result<()> {
        let len = self.cached_len.load(Ordering::Acquire);
        if len == 0 {
            *self.mmap.write() = None;
            self.mmap_len.store(0, Ordering::Release);
            return Ok(());
        }
        let m = unsafe { memmap2::Mmap::map(&self.file)? };
        let ml = m.len() as u64;
        *self.mmap.write() = Some(m);
        self.mmap_len.store(ml, Ordering::Release);
        Ok(())
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::unix::fs::FileExt;

    use super::FileStore;

    pub fn pread(store: &FileStore, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        store.file.read_exact_at(&mut buf, offset)?;
        Ok(buf)
    }

    pub fn pwrite(store: &FileStore, data: &[u8], offset: u64) -> io::Result<()> {
        store.file.write_all_at(data, offset)?;
        Ok(())
    }
}

#[cfg(not(unix))]
mod platform {
    use std::io::{self, Read, Seek, SeekFrom, Write};

    use super::FileStore;

    pub fn pread(store: &FileStore, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let _guard = store.write_lock.lock();
        let file = &store.file;
        let file = unsafe { &mut *(&*file as *const std::fs::File as *mut std::fs::File) };
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)?;
        Ok(buf)
    }

    pub fn pwrite(store: &FileStore, data: &[u8], offset: u64) -> io::Result<()> {
        let file = &store.file;
        let file = unsafe { &mut *(&*file as *const std::fs::File as *mut std::fs::File) };
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(data)?;
        Ok(())
    }
}

impl StaticStore for FileStore {
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let end = offset + len as u64;

        // Try mmap first (fast path: zero-copy from kernel page cache).
        if end <= self.mmap_len.load(Ordering::Acquire) {
            let guard = self.mmap.read();
            if let Some(ref mmap) = *guard {
                if end <= mmap.len() as u64 {
                    return Ok(mmap[offset as usize..end as usize].to_vec());
                }
            }
        }

        // Fallback: pread for data not yet covered by mmap.
        platform::pread(self, offset, len)
    }

    fn append(&self, data: &[u8]) -> io::Result<u64> {
        let _guard = self.write_lock.lock();
        let offset = self.cached_len.load(Ordering::Acquire);
        platform::pwrite(self, data, offset)?;
        self.cached_len.store(offset + data.len() as u64, Ordering::Release);
        Ok(offset)
    }

    fn len(&self) -> io::Result<u64> {
        Ok(self.cached_len.load(Ordering::Acquire))
    }

    fn sync(&self) -> io::Result<()> {
        self.file.sync_all()?;
        // Remap after sync so new data is visible via mmap.
        FileStore::remap(self)
    }

    fn remap(&self) -> io::Result<()> {
        FileStore::remap(self)
    }

    fn truncate(&self, len: u64) -> io::Result<()> {
        let _guard = self.write_lock.lock();
        // Drop mmap before truncating to avoid issues on some platforms.
        *self.mmap.write() = None;
        self.mmap_len.store(0, Ordering::Release);
        self.file.set_len(len)?;
        self.cached_len.store(len, Ordering::Release);
        // Remap if there's still data.
        if len > 0 {
            let m = unsafe { memmap2::Mmap::map(&self.file)? };
            let ml = m.len() as u64;
            *self.mmap.write() = Some(m);
            self.mmap_len.store(ml, Ordering::Release);
        }
        Ok(())
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

    fn remap(&self) -> io::Result<()> {
        match self {
            AnyStore::File(s) => s.remap(),
            AnyStore::Memory(_) => Ok(()),
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

        // Read before remap: falls back to pread.
        let data = store.read_at(0, 5).unwrap();
        assert_eq!(&data, b"hello");

        let data = store.read_at(5, 6).unwrap();
        assert_eq!(&data, b" world");

        // After remap, reads come from mmap.
        store.remap().unwrap();
        let data = store.read_at(0, 5).unwrap();
        assert_eq!(&data, b"hello");

        store.truncate(5).unwrap();
        assert_eq!(store.len().unwrap(), 5);
    }

    #[test]
    fn file_store_cached_len() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_len.dat");

        let store = FileStore::open(&path).unwrap();
        assert_eq!(store.len().unwrap(), 0);

        store.append(b"12345").unwrap();
        assert_eq!(store.len().unwrap(), 5);

        store.append(b"67890").unwrap();
        assert_eq!(store.len().unwrap(), 10);

        store.truncate(3).unwrap();
        assert_eq!(store.len().unwrap(), 3);
    }
}
