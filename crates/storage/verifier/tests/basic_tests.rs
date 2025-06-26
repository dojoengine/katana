//! Basic tests for the verifier crate.

use katana_verifier::{DatabaseVerifier, report::OverallStatus};

#[test]
fn test_verifier_creation() {
    let verifier = DatabaseVerifier::new();
    // Just ensure we can create a verifier without errors
    assert_eq!(std::mem::size_of_val(&verifier), std::mem::size_of::<DatabaseVerifier>());
}

#[test]
fn test_verifier_default() {
    let verifier = DatabaseVerifier::default();
    // Ensure default implementation works
    assert_eq!(std::mem::size_of_val(&verifier), std::mem::size_of::<DatabaseVerifier>());
}

#[test]
fn test_empty_report() {
    use katana_verifier::report::VerificationReport;
    
    let report = VerificationReport::empty();
    assert_eq!(report.total_count(), 0);
    assert_eq!(report.successful_count(), 0);
    assert_eq!(report.failed_count(), 0);
    assert_eq!(report.summary.overall_status, OverallStatus::Empty);
    assert!(!report.is_success()); // Empty is not success
    assert!(!report.has_failures());
}

#[test]
fn test_report_display() {
    use katana_verifier::report::VerificationReport;
    
    let report = VerificationReport::empty();
    let display_str = format!("{}", report);
    assert!(display_str.contains("Database Verification Report"));
    assert!(display_str.contains("Total verifiers: 0"));
}
