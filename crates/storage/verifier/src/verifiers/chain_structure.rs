//! Chain structure verification checks.

use anyhow::{Context, Result};
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use tracing::{debug, instrument};

use crate::error::VerificationError;
use crate::verifiers::Verifier;

/// Verifier for blockchain structure integrity.
/// 
/// This verifier checks:
/// - Parent-child block relationships
/// - Block hash computation correctness
/// - Block number sequence integrity
#[derive(Debug)]
pub struct ChainStructureVerifier;

impl Verifier for ChainStructureVerifier {
    fn name(&self) -> &'static str {
        "ChainStructureVerifier"
    }

    #[instrument(skip(self, database))]
    fn verify(&self, database: &DbProvider) -> Result<()> {
        self.verify_parent_child_relationships(database)?;
        self.verify_block_hashes(database)?;
        self.verify_block_number_sequence(database)?;
        Ok(())
    }

    #[instrument(skip(self, database))]
    fn verify_range(&self, database: &DbProvider, start_block: u64, end_block: u64) -> Result<()> {
        self.verify_parent_child_relationships_range(database, start_block, end_block)?;
        self.verify_block_hashes_range(database, start_block, end_block)?;
        self.verify_block_number_sequence_range(database, start_block, end_block)?;
        Ok(())
    }

    #[instrument(skip(self, database))]
    fn verify_sample(&self, database: &DbProvider, sample_rate: u64) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        if latest_block == 0 {
            debug!("No blocks found in database");
            return Ok(());
        }

        // Sample blocks at the specified rate
        for block_num in (0..=latest_block).step_by(sample_rate as usize) {
            self.verify_block_hash(database, block_num)?;
            
            // Verify parent relationship if not genesis block
            if block_num > 0 {
                self.verify_parent_child_relationship(database, block_num - 1, block_num)?;
            }
        }

        // Always verify the latest block
        if latest_block % sample_rate != 0 {
            self.verify_block_hash(database, latest_block)?;
        }

        Ok(())
    }
}

impl ChainStructureVerifier {
    /// Verify parent-child relationships for all blocks.
    fn verify_parent_child_relationships(&self, database: &DbProvider) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        if latest_block == 0 {
            debug!("Only genesis block exists, skipping parent-child verification");
            return Ok(());
        }

        debug!("Verifying parent-child relationships up to block {}", latest_block);

        for block_num in 1..=latest_block {
            self.verify_parent_child_relationship(database, block_num - 1, block_num)?;
        }

        Ok(())
    }

    /// Verify parent-child relationships within a specific range.
    fn verify_parent_child_relationships_range(&self, database: &DbProvider, start: u64, end: u64) -> Result<()> {
        debug!("Verifying parent-child relationships from {} to {}", start, end);

        // Start from max(1, start) since block 0 has no parent
        let start_block = if start == 0 { 1 } else { start };

        for block_num in start_block..=end {
            self.verify_parent_child_relationship(database, block_num - 1, block_num)?;
        }

        Ok(())
    }

    /// Verify the parent-child relationship between two consecutive blocks.
    fn verify_parent_child_relationship(&self, database: &DbProvider, parent_num: u64, child_num: u64) -> Result<()> {
        let parent_header = database
            .header_by_number(parent_num)
            .with_context(|| format!("Failed to query parent header for block {}", parent_num))?
            .with_context(|| format!("Missing parent header for block {}", parent_num))?;

        let child_header = database
            .header_by_number(child_num)
            .with_context(|| format!("Failed to query child header for block {}", child_num))?
            .with_context(|| format!("Missing child header for block {}", child_num))?;

        let parent_hash = parent_header.compute_hash();

        if child_header.parent_hash != parent_hash {
            return Err(VerificationError::ParentHashMismatch {
                block_number: child_num,
                expected: format!("{:#x}", parent_hash),
                actual: format!("{:#x}", child_header.parent_hash),
            }
            .into());
        }

        debug!("Block {} correctly references parent block {}", child_num, parent_num);
        Ok(())
    }

    /// Verify block hash computation for all blocks.
    fn verify_block_hashes(&self, database: &DbProvider) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        debug!("Verifying block hashes up to block {}", latest_block);

        for block_num in 0..=latest_block {
            self.verify_block_hash(database, block_num)?;
        }

        Ok(())
    }

    /// Verify block hash computation within a specific range.
    fn verify_block_hashes_range(&self, database: &DbProvider, start: u64, end: u64) -> Result<()> {
        debug!("Verifying block hashes from {} to {}", start, end);

        for block_num in start..=end {
            self.verify_block_hash(database, block_num)?;
        }

        Ok(())
    }

    /// Verify the block hash computation for a specific block.
    fn verify_block_hash(&self, database: &DbProvider, block_num: u64) -> Result<()> {
        let header = database
            .header_by_number(block_num)
            .with_context(|| format!("Failed to query header for block {}", block_num))?
            .with_context(|| format!("Missing header for block {}", block_num))?;

        let stored_hash = database
            .block_hash_by_num(block_num)
            .with_context(|| format!("Failed to query hash for block {}", block_num))?
            .with_context(|| format!("Missing hash for block {}", block_num))?;

        let computed_hash = header.compute_hash();

        if computed_hash != stored_hash {
            return Err(VerificationError::BlockHashMismatch {
                block_number: block_num,
                computed: format!("{:#x}", computed_hash),
                stored: format!("{:#x}", stored_hash),
            }
            .into());
        }

        debug!("Block {} hash verification passed: {:#x}", block_num, computed_hash);
        Ok(())
    }

    /// Verify block number sequence integrity.
    fn verify_block_number_sequence(&self, database: &DbProvider) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        debug!("Verifying block number sequence up to block {}", latest_block);

        for block_num in 0..=latest_block {
            let header = database
                .header_by_number(block_num)
                .with_context(|| format!("Failed to query header for block {}", block_num))?
                .with_context(|| format!("Missing header for block {}", block_num))?;

            if header.number != block_num {
                return Err(VerificationError::BlockSequenceGap {
                    previous: block_num,
                    current: header.number,
                }
                .into());
            }
        }

        debug!("Block number sequence verification passed");
        Ok(())
    }

    /// Verify block number sequence within a specific range.
    fn verify_block_number_sequence_range(&self, database: &DbProvider, start: u64, end: u64) -> Result<()> {
        debug!("Verifying block number sequence from {} to {}", start, end);

        for block_num in start..=end {
            let header = database
                .header_by_number(block_num)
                .with_context(|| format!("Failed to query header for block {}", block_num))?
                .with_context(|| format!("Missing header for block {}", block_num))?;

            if header.number != block_num {
                return Err(VerificationError::BlockSequenceGap {
                    previous: block_num,
                    current: header.number,
                }
                .into());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_name() {
        let verifier = ChainStructureVerifier;
        assert_eq!(verifier.name(), "ChainStructureVerifier");
    }
}
