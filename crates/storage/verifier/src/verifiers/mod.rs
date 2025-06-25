//! Individual verifier implementations.

use anyhow::Result;
use katana_provider::providers::db::DbProvider;

pub mod basic;
pub mod chain_structure;
pub mod commitment;
pub mod state_trie;

pub use basic::BasicIntegrityVerifier;
pub use chain_structure::ChainStructureVerifier;
pub use commitment::CommitmentVerifier;
pub use state_trie::StateTrieVerifier;

/// Trait for database verifiers.
pub trait Verifier: std::fmt::Debug + Send + Sync {
    /// Get the name of this verifier.
    fn name(&self) -> &'static str;

    /// Verify the entire database.
    fn verify(&self, database: &DbProvider) -> Result<()>;

    /// Verify a specific block range.
    /// Default implementation calls verify() - verifiers can override for efficiency.
    fn verify_range(&self, database: &DbProvider, start_block: u64, end_block: u64) -> Result<()> {
        // Default implementation ignores the range and verifies everything
        // Individual verifiers can override this for more efficient range verification
        let _ = (start_block, end_block);
        self.verify(database)
    }

    /// Verify using sampling (every nth block).
    /// Default implementation calls verify() - verifiers can override for efficiency.
    fn verify_sample(&self, database: &DbProvider, sample_rate: u64) -> Result<()> {
        // Default implementation ignores the sample rate and verifies everything
        // Individual verifiers can override this for more efficient sampling
        let _ = sample_rate;
        self.verify(database)
    }
}
