use amd_tee_registry_client::{AmdAttestationProver, Error, ProverConfig, ATTESTATION_REPORT_SIZE};

#[test]
fn test_prover_creation() {
    let config = ProverConfig::new(Some("key".to_string()), None, false);
    let prover = AmdAttestationProver::new(config);
    assert!(prover.config().has_network_key());
}

#[tokio::test]
async fn test_invalid_report_size() {
    let prover = AmdAttestationProver::from_env();
    let invalid_report = vec![0u8; 100]; // Wrong size

    let result = prover.prove(&invalid_report).await;
    assert!(result.is_err());

    match result {
        Err(Error::InvalidReportSize { expected, actual }) => {
            assert_eq!(expected, ATTESTATION_REPORT_SIZE);
            assert_eq!(actual, 100);
        }
        _ => panic!("Expected InvalidReportSize error"),
    }
}
