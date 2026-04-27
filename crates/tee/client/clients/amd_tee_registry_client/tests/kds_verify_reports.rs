mod common;

use std::time::Duration;

use amd_sev_snp_attestation_verifier::{
    stub::VerificationResult, verify_attestation, AttestationReport,
};
use common::{
    build_verifier_input, fetch_kds_chain_with_timeout, fixture_paths_kds, load_report_bytes,
    pick_valid_timestamp, print_journal, print_report,
};
use x509_verifier_rust_crypto::CertChain;

#[test]
fn verify_reports_with_kds() -> anyhow::Result<()> {
    for (i, path) in fixture_paths_kds().iter().enumerate() {
        println!(
            "Verifying report #{} / {} : {}",
            i + 1,
            fixture_paths_kds().len(),
            path.file_name().unwrap().to_string_lossy(),
        );
        let report_bytes = load_report_bytes(&path)?;
        let report = AttestationReport::from_bytes(&report_bytes)?;

        let kds_chain = fetch_kds_chain_with_timeout(report, Duration::from_secs(20))?;
        let cert_chain = CertChain::parse_rev(&kds_chain)?;
        let timestamp = pick_valid_timestamp(&cert_chain)?;

        let input = build_verifier_input(timestamp, &report_bytes, &cert_chain);
        let journal = verify_attestation(input)?;

        let fixture_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        print_report(&report, &fixture_name);
        print_journal(&journal);

        assert!(matches!(journal.result, VerificationResult::Success));
        assert_eq!(journal.rawReport.len(), report_bytes.len());
        println!("\n\n=========================================\n\n");
    }

    Ok(())
}
