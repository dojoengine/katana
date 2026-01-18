use katana_tee_client::ProverConfig;

#[test]
fn test_config_from_env() {
    std::env::set_var("NETWORK_PRIVATE_KEY", "test_key");
    std::env::set_var("SKIP_TIME_VALIDITY_CHECK", "true");

    let config = ProverConfig::from_env();
    assert_eq!(config.private_key, Some("test_key".to_string()));
    assert!(config.skip_time_validity_check);

    std::env::remove_var("NETWORK_PRIVATE_KEY");
    std::env::remove_var("SKIP_TIME_VALIDITY_CHECK");
}
