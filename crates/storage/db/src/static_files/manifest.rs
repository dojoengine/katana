use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

const MANIFEST_VERSION: u32 = 1;

/// On-disk manifest tracking committed state for crash recovery.
///
/// After a crash, any data/index beyond the manifest's committed counts is truncated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    /// The number of blocks committed (i.e., valid block keys are `0..latest_block_count`).
    pub latest_block_count: u64,
    /// The number of transactions committed.
    pub latest_tx_count: u64,
}

impl Default for Manifest {
    fn default() -> Self {
        Self { version: MANIFEST_VERSION, latest_block_count: 0, latest_tx_count: 0 }
    }
}

impl Manifest {
    /// Read the manifest from a file. Returns `None` if the file doesn't exist.
    pub fn read_from_file(path: &Path) -> io::Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let manifest: Self = serde_json::from_str(&contents)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(Some(manifest))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Atomically write the manifest to a file.
    ///
    /// Writes to a temporary file first, then renames to ensure atomicity.
    pub fn write_to_file(&self, path: &Path) -> io::Result<()> {
        let contents = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Write to a temporary file alongside the target, then rename.
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, contents)?;
        std::fs::rename(&tmp_path, path)?;

        Ok(())
    }

    /// Returns the latest block number, or `None` if no blocks have been committed.
    pub fn latest_block_number(&self) -> Option<u64> {
        if self.latest_block_count == 0 {
            None
        } else {
            Some(self.latest_block_count - 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrip_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");

        let manifest = Manifest { version: 1, latest_block_count: 100, latest_tx_count: 5000 };

        manifest.write_to_file(&path).unwrap();

        let loaded = Manifest::read_from_file(&path).unwrap().unwrap();
        assert_eq!(manifest, loaded);
    }

    #[test]
    fn manifest_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");

        let result = Manifest::read_from_file(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn latest_block_number() {
        let m = Manifest::default();
        assert_eq!(m.latest_block_number(), None);

        let m = Manifest { latest_block_count: 1, ..Default::default() };
        assert_eq!(m.latest_block_number(), Some(0));

        let m = Manifest { latest_block_count: 100, ..Default::default() };
        assert_eq!(m.latest_block_number(), Some(99));
    }
}
