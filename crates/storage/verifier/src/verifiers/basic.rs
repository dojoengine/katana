//! Basic integrity verification checks.

use anyhow::{Context, Result};
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use katana_provider::traits::transaction::TransactionProvider;
use tracing::{debug, instrument};

use crate::error::VerificationError;
use crate::verifiers::Verifier;

/// Verifier for basic database integrity.
/// 
/// This verifier checks:
/// - Database consistency (no missing blocks or transactions)
/// - Block-transaction relationships
/// - Basic data structure integrity
#[derive(Debug)]
pub struct BasicIntegrityVerifier;

impl Verifier for BasicIntegrityVerifier {
    fn name(&self) -> &'static str {
        "BasicIntegrityVerifier"
    }

    #[instrument(skip(self, database))]
    fn verify(&self, database: &DbProvider) -> Result<()> {
        self.verify_block_continuity(database)?;
        self.verify_transaction_continuity(database)?;
        self.verify_block_transaction_relationships(database)?;
        Ok(())
    }

    #[instrument(skip(self, database))]
    fn verify_range(&self, database: &DbProvider, start_block: u64, end_block: u64) -> Result<()> {
        self.verify_block_continuity_range(database, start_block, end_block)?;
        self.verify_transaction_continuity_range(database, start_block, end_block)?;
        self.verify_block_transaction_relationships_range(database, start_block, end_block)?;
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
        let mut sampled_blocks = Vec::new();
        for block_num in (0..=latest_block).step_by(sample_rate as usize) {
            sampled_blocks.push(block_num);
        }
        
        // Always include the latest block
        if !sampled_blocks.contains(&latest_block) {
            sampled_blocks.push(latest_block);
        }

        debug!("Sampling {} blocks out of {}", sampled_blocks.len(), latest_block + 1);

        for block_num in sampled_blocks {
            self.verify_block_exists(database, block_num)?;
            self.verify_block_transaction_relationship(database, block_num)?;
        }

        Ok(())
    }
}

impl BasicIntegrityVerifier {
    /// Verify that all blocks exist in sequence without gaps.
    fn verify_block_continuity(&self, database: &DbProvider) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        debug!("Verifying block continuity up to block {}", latest_block);

        for block_num in 0..=latest_block {
            self.verify_block_exists(database, block_num)?;
        }

        Ok(())
    }

    /// Verify block continuity within a specific range.
    fn verify_block_continuity_range(&self, database: &DbProvider, start: u64, end: u64) -> Result<()> {
        debug!("Verifying block continuity from {} to {}", start, end);

        for block_num in start..=end {
            self.verify_block_exists(database, block_num)?;
        }

        Ok(())
    }

    /// Verify that a specific block exists.
    fn verify_block_exists(&self, database: &DbProvider, block_num: u64) -> Result<()> {
        let header = database
            .header_by_number(block_num)
            .with_context(|| format!("Failed to query header for block {}", block_num))?;

        if header.is_none() {
            return Err(VerificationError::MissingBlockData {
                block_number: block_num,
                data_type: "header".to_string(),
            }
            .into());
        }

        let hash = database
            .block_hash_by_num(block_num)
            .with_context(|| format!("Failed to query hash for block {}", block_num))?;

        if hash.is_none() {
            return Err(VerificationError::MissingBlockData {
                block_number: block_num,
                data_type: "hash".to_string(),
            }
            .into());
        }

        Ok(())
    }

    /// Verify transaction continuity (no missing transactions).
    fn verify_transaction_continuity(&self, database: &DbProvider) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        debug!("Verifying transaction continuity up to block {}", latest_block);

        for block_num in 0..=latest_block {
            self.verify_block_transactions_exist(database, block_num)?;
        }

        Ok(())
    }

    /// Verify transaction continuity within a specific range.
    fn verify_transaction_continuity_range(&self, database: &DbProvider, start: u64, end: u64) -> Result<()> {
        debug!("Verifying transaction continuity from {} to {}", start, end);

        for block_num in start..=end {
            self.verify_block_transactions_exist(database, block_num)?;
        }

        Ok(())
    }

    /// Verify that transactions exist for a specific block.
    fn verify_block_transactions_exist(&self, database: &DbProvider, block_num: u64) -> Result<()> {
        let transactions = database
            .transactions_by_block(block_num.into())
            .with_context(|| format!("Failed to query transactions for block {}", block_num))?;

        if transactions.is_none() {
            return Err(VerificationError::MissingBlockData {
                block_number: block_num,
                data_type: "transactions".to_string(),
            }
            .into());
        }

        Ok(())
    }

    /// Verify block-transaction relationships.
    fn verify_block_transaction_relationships(&self, database: &DbProvider) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        debug!("Verifying block-transaction relationships up to block {}", latest_block);

        for block_num in 0..=latest_block {
            self.verify_block_transaction_relationship(database, block_num)?;
        }

        Ok(())
    }

    /// Verify block-transaction relationships within a specific range.
    fn verify_block_transaction_relationships_range(&self, database: &DbProvider, start: u64, end: u64) -> Result<()> {
        debug!("Verifying block-transaction relationships from {} to {}", start, end);

        for block_num in start..=end {
            self.verify_block_transaction_relationship(database, block_num)?;
        }

        Ok(())
    }

    /// Verify the relationship between a block and its transactions.
    fn verify_block_transaction_relationship(&self, database: &DbProvider, block_num: u64) -> Result<()> {
        let header = database
            .header_by_number(block_num)
            .with_context(|| format!("Failed to query header for block {}", block_num))?
            .with_context(|| format!("Missing header for block {}", block_num))?;

        let transactions = database
            .transactions_by_block(block_num.into())
            .with_context(|| format!("Failed to query transactions for block {}", block_num))?
            .unwrap_or_default();

        // Verify transaction count matches header
        if header.transaction_count as usize != transactions.len() {
            return Err(VerificationError::TransactionCountMismatch {
                block_number: block_num,
                expected: header.transaction_count as usize,
                actual: transactions.len(),
            }
            .into());
        }

        debug!("Block {} has correct transaction count: {}", block_num, transactions.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests would require setting up a test database
    // For now, we'll just test the structure

    #[test]
    fn test_verifier_name() {
        let verifier = BasicIntegrityVerifier;
        assert_eq!(verifier.name(), "BasicIntegrityVerifier");
    }
}
