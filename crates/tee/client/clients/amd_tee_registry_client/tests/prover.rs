use amd_tee_registry_client::{
    AmdAttestationProver, Error, ProverConfig, StarknetRegistryClient, ATTESTATION_REPORT_SIZE,
};
use starknet::core::types::Felt;

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

    // Create a dummy registry client (won't be used - error happens before registry query)
    let dummy_registry = StarknetRegistryClient::new("http://localhost:5050", Felt::ZERO);

    let result = prover.prove(&invalid_report, &dummy_registry).await;
    assert!(result.is_err());

    match result {
        Err(Error::InvalidReportSize { expected, actual }) => {
            assert_eq!(expected, ATTESTATION_REPORT_SIZE);
            assert_eq!(actual, 100);
        }
        _ => panic!("Expected InvalidReportSize error"),
    }
}
