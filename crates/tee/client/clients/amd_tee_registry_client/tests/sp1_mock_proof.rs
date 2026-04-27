mod common;

use std::time::Duration;

use alloy_primitives::Bytes;
use amd_sev_snp_attestation_prover::{AmdSevSnpProver, ProverConfig, RawProofType};
use amd_sev_snp_attestation_verifier::{stub::VerifierJournal, AttestationReport};
use common::{
    fetch_kds_chain_with_timeout, fixture_paths_kds, load_report_bytes, pick_valid_timestamp,
    print_journal, print_report,
};
use x509_verifier_rust_crypto::CertChain;

#[test]
#[ignore] // Requires network access to AMD KDS and valid cert chains
fn sp1_mock_execution_only_proof() -> anyhow::Result<()> {
    std::env::set_var("SP1_PROVER", "mock");

    for path in fixture_paths_kds() {
        let report_bytes = load_report_bytes(&path)?;
        let report = AttestationReport::from_bytes(&report_bytes)?;

        let kds_chain = fetch_kds_chain_with_timeout(report, Duration::from_secs(20))?;
        let cert_chain = CertChain::parse_rev(&kds_chain)?;
        let timestamp = pick_valid_timestamp(&cert_chain)?;

        let prover = AmdSevSnpProver::new(ProverConfig::sp1(), None);
        // Pass the exact KDS chain we fetched for timestamp selection.
        // Re-fetching inside the prover can race AMD KDS rotations and make tests flaky.
        let input = prover.prepare_verifier_input(
            timestamp,
            Bytes::from(report_bytes.clone()),
            Some(kds_chain),
        )?;

        let raw_proof = prover
            .verifier
            .gen_proof(&input, RawProofType::Composite, None)?;

        assert!(!raw_proof.journal.is_empty());
        let onchain = prover.verifier.onchain_proof(&raw_proof)?;
        assert!(onchain.is_empty());

        let journal = VerifierJournal::decode(raw_proof.journal.as_ref())?;
        let fixture_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        print_report(&report, &fixture_name);
        print_journal(&journal);
    }

    Ok(())
}
