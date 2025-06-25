//! Verification report structures for tracking verification results.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The result of a single verification check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationResult {
    /// Verification passed successfully.
    Success,
    /// Verification failed with an error message.
    Failed { error: String },
}

/// Comprehensive report of database verification results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    /// Results for each verifier that was run.
    pub results: HashMap<String, VerificationResult>,
    /// Summary statistics.
    pub summary: VerificationSummary,
}

/// Summary statistics for the verification report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSummary {
    /// Total number of verifiers run.
    pub total_verifiers: usize,
    /// Number of successful verifications.
    pub successful: usize,
    /// Number of failed verifications.
    pub failed: usize,
    /// Overall verification status.
    pub overall_status: OverallStatus,
}

/// Overall verification status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OverallStatus {
    /// All verifications passed.
    Success,
    /// Some verifications failed.
    Failed,
    /// No verifications were run.
    Empty,
}

impl VerificationReport {
    /// Create a new verification report from results.
    pub fn new(results: HashMap<String, VerificationResult>) -> Self {
        let total_verifiers = results.len();
        let successful = results.values().filter(|r| matches!(r, VerificationResult::Success)).count();
        let failed = total_verifiers - successful;

        let overall_status = if total_verifiers == 0 {
            OverallStatus::Empty
        } else if failed == 0 {
            OverallStatus::Success
        } else {
            OverallStatus::Failed
        };

        let summary = VerificationSummary {
            total_verifiers,
            successful,
            failed,
            overall_status,
        };

        Self { results, summary }
    }

    /// Create an empty verification report.
    pub fn empty() -> Self {
        Self::new(HashMap::new())
    }

    /// Check if all verifications passed.
    pub fn is_success(&self) -> bool {
        self.summary.overall_status == OverallStatus::Success
    }

    /// Check if any verifications failed.
    pub fn has_failures(&self) -> bool {
        self.summary.failed > 0
    }

    /// Get the number of successful verifications.
    pub fn successful_count(&self) -> usize {
        self.summary.successful
    }

    /// Get the number of failed verifications.
    pub fn failed_count(&self) -> usize {
        self.summary.failed
    }

    /// Get the total number of verifications run.
    pub fn total_count(&self) -> usize {
        self.summary.total_verifiers
    }

    /// Get all failed verifier names and their error messages.
    pub fn failures(&self) -> Vec<(&String, &String)> {
        self.results
            .iter()
            .filter_map(|(name, result)| {
                if let VerificationResult::Failed { error } = result {
                    Some((name, error))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all successful verifier names.
    pub fn successes(&self) -> Vec<&String> {
        self.results
            .iter()
            .filter_map(|(name, result)| {
                if matches!(result, VerificationResult::Success) {
                    Some(name)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Convert the report to a JSON string.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Create a report from a JSON string.
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

impl std::fmt::Display for VerificationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Database Verification Report")?;
        writeln!(f, "============================")?;
        writeln!(f, "Total verifiers: {}", self.summary.total_verifiers)?;
        writeln!(f, "Successful: {}", self.summary.successful)?;
        writeln!(f, "Failed: {}", self.summary.failed)?;
        writeln!(f, "Overall status: {:?}", self.summary.overall_status)?;
        writeln!(f)?;

        if !self.successes().is_empty() {
            writeln!(f, "Successful verifications:")?;
            for name in self.successes() {
                writeln!(f, "  ✓ {}", name)?;
            }
            writeln!(f)?;
        }

        if !self.failures().is_empty() {
            writeln!(f, "Failed verifications:")?;
            for (name, error) in self.failures() {
                writeln!(f, "  ✗ {}: {}", name, error)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_report() {
        let report = VerificationReport::empty();
        assert_eq!(report.total_count(), 0);
        assert_eq!(report.successful_count(), 0);
        assert_eq!(report.failed_count(), 0);
        assert_eq!(report.summary.overall_status, OverallStatus::Empty);
    }

    #[test]
    fn test_successful_report() {
        let mut results = HashMap::new();
        results.insert("test1".to_string(), VerificationResult::Success);
        results.insert("test2".to_string(), VerificationResult::Success);

        let report = VerificationReport::new(results);
        assert_eq!(report.total_count(), 2);
        assert_eq!(report.successful_count(), 2);
        assert_eq!(report.failed_count(), 0);
        assert!(report.is_success());
        assert!(!report.has_failures());
        assert_eq!(report.summary.overall_status, OverallStatus::Success);
    }

    #[test]
    fn test_failed_report() {
        let mut results = HashMap::new();
        results.insert("test1".to_string(), VerificationResult::Success);
        results.insert("test2".to_string(), VerificationResult::Failed { 
            error: "Test error".to_string() 
        });

        let report = VerificationReport::new(results);
        assert_eq!(report.total_count(), 2);
        assert_eq!(report.successful_count(), 1);
        assert_eq!(report.failed_count(), 1);
        assert!(!report.is_success());
        assert!(report.has_failures());
        assert_eq!(report.summary.overall_status, OverallStatus::Failed);
    }

    #[test]
    fn test_json_serialization() {
        let mut results = HashMap::new();
        results.insert("test1".to_string(), VerificationResult::Success);
        
        let report = VerificationReport::new(results);
        let json = report.to_json().unwrap();
        let deserialized = VerificationReport::from_json(&json).unwrap();
        
        assert_eq!(report.total_count(), deserialized.total_count());
        assert_eq!(report.is_success(), deserialized.is_success());
    }
}
