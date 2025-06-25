//! Katana database integrity verification tool.
//!
//! This crate provides comprehensive verification of Katana blockchain database integrity.
//! It can verify block structure, transaction consistency, state trie correctness,
//! and execution determinism independently of any migration or other processes.

pub mod error;
pub mod report;
pub mod verifiers;

use std::collections::HashMap;

use anyhow::{Context, Result};
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::block::BlockNumberProvider;
use tracing::{info, instrument};

use crate::error::VerificationError;
use crate::report::{VerificationReport, VerificationResult};
use crate::verifiers::{
    BasicIntegrityVerifier, ChainStructureVerifier, CommitmentVerifier, StateTrieVerifier,
    Verifier,
};

/// Main verification orchestrator that runs all verification checks.
#[derive(Debug)]
pub struct DatabaseVerifier {
    verifiers: Vec<Box<dyn Verifier>>,
}

impl DatabaseVerifier {
    /// Create a new database verifier with all default verifiers.
    pub fn new() -> Self {
        let verifiers: Vec<Box<dyn Verifier>> = vec![
            Box::new(BasicIntegrityVerifier),
            Box::new(ChainStructureVerifier),
            Box::new(StateTrieVerifier),
            Box::new(CommitmentVerifier),
        ];

        Self { verifiers }
    }

    /// Create a new database verifier with custom verifiers.
    pub fn with_verifiers(verifiers: Vec<Box<dyn Verifier>>) -> Self {
        Self { verifiers }
    }

    /// Add a verifier to the verification process.
    pub fn add_verifier(&mut self, verifier: Box<dyn Verifier>) {
        self.verifiers.push(verifier);
    }

    /// Run all verification checks on the given database.
    #[instrument(skip(self, database))]
    pub fn verify_database(&self, database: &DbProvider) -> Result<VerificationReport> {
        info!("Starting database verification with {} verifiers", self.verifiers.len());

        let mut results = HashMap::new();

        for verifier in &self.verifiers {
            let name = verifier.name();
            info!("Running verifier: {}", name);

            let result = match verifier.verify(database) {
                Ok(()) => {
                    info!("Verifier '{}' completed successfully", name);
                    VerificationResult::Success
                }
                Err(e) => {
                    let error_msg = format!("Verifier '{}' failed: {}", name, e);
                    tracing::error!("{}", error_msg);
                    VerificationResult::Failed { error: e.to_string() }
                }
            };

            results.insert(name.to_string(), result);
        }

        let report = VerificationReport::new(results);
        info!(
            "Database verification completed. Success: {}, Failed: {}",
            report.successful_count(),
            report.failed_count()
        );

        Ok(report)
    }

    /// Run verification on a specific block range.
    #[instrument(skip(self, database))]
    pub fn verify_block_range(
        &self,
        database: &DbProvider,
        start_block: u64,
        end_block: u64,
    ) -> Result<VerificationReport> {
        info!("Starting block range verification: {} to {}", start_block, end_block);

        // Validate block range
        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        if end_block > latest_block {
            return Err(VerificationError::InvalidBlockRange {
                start: start_block,
                end: end_block,
                latest: latest_block,
            }
            .into());
        }

        let mut results = HashMap::new();

        for verifier in &self.verifiers {
            let name = verifier.name();
            info!("Running verifier '{}' on block range {} to {}", name, start_block, end_block);

            let result = match verifier.verify_range(database, start_block, end_block) {
                Ok(()) => {
                    info!("Verifier '{}' completed successfully for block range", name);
                    VerificationResult::Success
                }
                Err(e) => {
                    let error_msg = format!("Verifier '{}' failed on block range: {}", name, e);
                    tracing::error!("{}", error_msg);
                    VerificationResult::Failed { error: e.to_string() }
                }
            };

            results.insert(name.to_string(), result);
        }

        let report = VerificationReport::new(results);
        info!(
            "Block range verification completed. Success: {}, Failed: {}",
            report.successful_count(),
            report.failed_count()
        );

        Ok(report)
    }

    /// Run a quick verification that samples blocks instead of checking all blocks.
    /// This is useful for large databases where full verification would be too slow.
    #[instrument(skip(self, database))]
    pub fn verify_sample(
        &self,
        database: &DbProvider,
        sample_rate: u64,
    ) -> Result<VerificationReport> {
        info!("Starting sample verification with rate 1/{}", sample_rate);

        let latest_block = database.latest_number().context("Failed to get latest block number")?;
        
        if latest_block == 0 {
            info!("No blocks found in database");
            return Ok(VerificationReport::empty());
        }

        let mut results = HashMap::new();

        for verifier in &self.verifiers {
            let name = verifier.name();
            info!("Running sampled verifier: {}", name);

            let result = match verifier.verify_sample(database, sample_rate) {
                Ok(()) => {
                    info!("Sampled verifier '{}' completed successfully", name);
                    VerificationResult::Success
                }
                Err(e) => {
                    let error_msg = format!("Sampled verifier '{}' failed: {}", name, e);
                    tracing::error!("{}", error_msg);
                    VerificationResult::Failed { error: e.to_string() }
                }
            };

            results.insert(name.to_string(), result);
        }

        let report = VerificationReport::new(results);
        info!(
            "Sample verification completed. Success: {}, Failed: {}",
            report.successful_count(),
            report.failed_count()
        );

        Ok(report)
    }
}

impl Default for DatabaseVerifier {
    fn default() -> Self {
        Self::new()
    }
}
