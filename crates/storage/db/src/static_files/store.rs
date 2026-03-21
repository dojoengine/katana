use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::{Mutex, RwLock};

/// Low-level byte storage backend for static file columns.
///
/// Implementations must be append-only: `append()` always writes at the end, and the
/// returned offset is monotonically increasing. Overwrites within the committed range
/// are never performed (only `truncate` modifies existing data, for crash recovery).
///
/// Two implementations exist:
/// - [`FileStore`] — production, backed by real files with pread/pwrite + mmap
/// - [`MemoryStore`] — tests and in-memory mode, backed by `Vec<u8>`
pub trait StaticStore: Send + Sync + 'static {
    /// Read `len` bytes starting at `offset`. Returns an error if the range is past EOF.
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>>;
    /// Append `data` at the end. Returns the byte offset where data was written.
    /// The store is append-only — this never overwrites existing data.
    fn append(&self, data: &[u8]) -> io::Result<u64>;
    /// Current length in bytes.
    fn len(&self) -> io::Result<u64>;
    /// Flush buffered data to durable storage and refresh read caches (e.g., mmap).
    /// Callers should call this before making MDBX pointers visible via commit.
    fn sync(&self) -> io::Result<()>;
    /// Truncate to the given byte length. Used only during crash recovery to discard
    /// orphaned data beyond the MDBX-committed position. Invalidates any stale mmap.
    fn truncate(&self, len: u64) -> io::Result<()>;
    /// Refresh read caches (e.g., mmap) to cover data written since the last remap.
    /// Lightweight (no disk I/O beyond the mmap syscall). Called automatically by
    /// [`sync`] and by [`MutableProvider::commit`] after MDBX commit.
    fn remap(&self) -> io::Result<()> {
        Ok(())
    }
    /// Hint that `additional` bytes will be appended soon. Pre-allocates the write
    /// buffer to avoid reallocation during a batch.
    fn reserve(&self, _additional: usize) -> io::Result<()> {
        Ok(())
    }
}

/// File-backed store using `pread`/`pwrite` for concurrent I/O, with mmap for reads.
///
/// - Reads use mmap when the data is within the mapped region, falling back to pread for data
///   written after the last remap.
/// - Writes use `pwrite` under a mutex for serialized appends.
/// - The mmap is refreshed on `remap()` to cover newly-written data.
///
/// ## Invariants
///
/// - `cached_len` always equals the logical file length (on-disk length + buffered data).
/// - `mmap_len` equals the length of the current mmap mapping. Data between `mmap_len` and
///   `cached_len` is in the write buffer and served by the buffer on read, or via pread if already
///   flushed but not yet remapped.
/// - The write buffer starts at byte offset `buf_start` in the file. `buf_start + buf.len()` always
///   equals `cached_len`.
pub struct FileStore {
    file: File,
    /// Serializes append writes and protects the write buffer.
    write_state: Mutex<WriteState>,
    /// Cached file length (includes buffered data not yet flushed).
    cached_len: AtomicU64,
    /// Memory-mapped read view. RwLock allows concurrent reads, exclusive remap.
    mmap: RwLock<Option<memmap2::Mmap>>,
    /// Length covered by the current mmap.
    mmap_len: AtomicU64,
}

struct WriteState {
    /// Buffered writes not yet flushed to disk.
    buf: Vec<u8>,
    /// The file offset where the buffer starts (i.e., the on-disk file length).
    buf_start: u64,
}

impl FileStore {
    /// Open or create a file at the given path. If the file already exists, its contents
    /// are preserved and an mmap is created covering the existing data. The write buffer
    /// starts empty at the current file length.
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
            write_state: Mutex::new(WriteState {
                buf: Vec::with_capacity(64 * 1024),
                buf_start: len,
            }),
            cached_len: AtomicU64::new(len),
            mmap: RwLock::new(mmap),
            mmap_len: AtomicU64::new(mmap_len),
        })
    }

    /// Flush the write buffer to disk in a single pwrite.
    fn flush_buf(&self) -> io::Result<()> {
        let mut ws = self.write_state.lock();
        if ws.buf.is_empty() {
            return Ok(());
        }
        platform::pwrite(self, &ws.buf, ws.buf_start)?;
        ws.buf_start += ws.buf.len() as u64;
        ws.buf.clear();
        Ok(())
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

        // Check the write buffer for recently-appended data not yet flushed.
        {
            let ws = self.write_state.lock();
            if !ws.buf.is_empty()
                && offset >= ws.buf_start
                && end <= ws.buf_start + ws.buf.len() as u64
            {
                let buf_offset = (offset - ws.buf_start) as usize;
                return Ok(ws.buf[buf_offset..buf_offset + len].to_vec());
            }
        }

        // Fallback: pread for data not covered by mmap or buffer.
        platform::pread(self, offset, len)
    }

    fn append(&self, data: &[u8]) -> io::Result<u64> {
        let mut ws = self.write_state.lock();
        let offset = self.cached_len.load(Ordering::Acquire);
        ws.buf.extend_from_slice(data);
        self.cached_len.store(offset + data.len() as u64, Ordering::Release);

        // Auto-flush when buffer gets large.
        if ws.buf.len() >= 256 * 1024 {
            platform::pwrite(self, &ws.buf, ws.buf_start)?;
            ws.buf_start += ws.buf.len() as u64;
            ws.buf.clear();
        }

        Ok(offset)
    }

    fn len(&self) -> io::Result<u64> {
        Ok(self.cached_len.load(Ordering::Acquire))
    }

    fn sync(&self) -> io::Result<()> {
        self.flush_buf()?;
        self.file.sync_all()?;
        FileStore::remap(self)
    }

    fn remap(&self) -> io::Result<()> {
        self.flush_buf()?;
        FileStore::remap(self)
    }

    fn reserve(&self, additional: usize) -> io::Result<()> {
        let mut ws = self.write_state.lock();
        ws.buf.reserve(additional);
        Ok(())
    }

    fn truncate(&self, len: u64) -> io::Result<()> {
        // Discard write buffer.
        {
            let mut ws = self.write_state.lock();
            ws.buf.clear();
            ws.buf_start = len;
        }
        // Drop mmap before truncating.
        *self.mmap.write() = None;
        self.mmap_len.store(0, Ordering::Release);
        self.file.set_len(len)?;
        self.cached_len.store(len, Ordering::Release);
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

    fn reserve(&self, additional: usize) -> io::Result<()> {
        match self {
            AnyStore::File(s) => s.reserve(additional),
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
