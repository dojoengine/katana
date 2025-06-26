//! State trie verification checks.

use anyhow::{Context, Result};
use katana_db::abstraction::Database;
use katana_db::trie::TrieDbFactory;
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::block::{BlockNumberProvider, HeaderProvider};
use katana_provider::traits::state::{StateFactoryProvider, StateRootProvider};
use tracing::{debug, instrument};

use crate::error::VerificationError;
use crate::verifiers::Verifier;

/// Verifier for state trie integrity.
///
/// This verifier checks that the computed state root from the database's state provider
/// matches the state root stored in the block header. The verification is performed
/// only on the last block in the verification range for efficiency.
///
/// The state root is computed using the StateRootProvider trait which combines:
/// - Contracts trie root
/// - Classes trie root
///
/// This approach is much more efficient than verifying every block individually.
#[derive(Debug)]
pub struct StateTrieVerifier;

impl Verifier for StateTrieVerifier {
    fn name(&self) -> &'static str {
        "StateTrieVerifier"
    }

    #[instrument(skip(self, database))]
    fn verify(&self, database: &DbProvider) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;

        if latest_block == 0 {
            debug!("No blocks found in database");
            return Ok(());
        }

        // Only verify the state root of the latest block
        self.verify_state_root_at_block(database, latest_block)?;
        Ok(())
    }

    #[instrument(skip(self, database))]
    fn verify_range(&self, database: &DbProvider, start_block: u64, end_block: u64) -> Result<()> {
        // Only verify the state root of the last block in the range
        self.verify_state_root_at_block(database, end_block)?;
        Ok(())
    }

    #[instrument(skip(self, database))]
    fn verify_sample(&self, database: &DbProvider, sample_rate: u64) -> Result<()> {
        let latest_block = database.latest_number().context("Failed to get latest block number")?;

        if latest_block == 0 {
            debug!("No blocks found in database");
            return Ok(());
        }

        // For sampling, only verify the state root of the last block
        self.verify_state_root_at_block(database, latest_block)?;

        Ok(())
    }
}

impl StateTrieVerifier {
    /// Verify the state root for a specific block by comparing the header's state root
    /// with the computed state root from the state provider.
    fn verify_state_root_at_block(&self, database: &DbProvider, block_num: u64) -> Result<()> {
        debug!("Verifying state root for block {}", block_num);

        // Get the block header to compare against
        let header = database
            .header_by_number(block_num)
            .with_context(|| format!("Failed to query header for block {}", block_num))?
            .with_context(|| format!("Missing header for block {}", block_num))?;

        // Get historical state provider for this block
        let state_provider = database
            .historical(block_num.into())
            .with_context(|| format!("Failed to get historical state for block {}", block_num))?
            .with_context(|| format!("Missing historical state for block {}", block_num))?;

        // Compute the state root using the StateRootProvider trait
        let computed_state_root = state_provider
            .state_root()
            .with_context(|| format!("Failed to compute state root for block {}", block_num))?;

        let tx = database.db().tx_mut()?;
        let trie = TrieDbFactory::new(&tx).historical(block_num).unwrap();

        let classes_trie = trie.classes_trie();
        let contracts_trie = trie.contracts_trie();

        // Compare the computed state root with the header's state root
        if header.state_root != computed_state_root {
            return Err(VerificationError::StateRootMismatch {
                block_number: block_num,
                header: format!("{:#x}", header.state_root),
                computed: format!("{:#x}", computed_state_root),
            }
            .into());
        }

        debug!(
            "State root verification passed for block {}: {:#x}",
            block_num, computed_state_root
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_name() {
        let verifier = StateTrieVerifier;
        assert_eq!(verifier.name(), "StateTrieVerifier");
    }
}
