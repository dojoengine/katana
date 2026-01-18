use amd_tee_registry_client::ProverConfig;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn test_config_from_env() {
    let _guard = ENV_LOCK.lock().expect("ENV_LOCK poisoned");
    // Set test environment
    std::env::set_var("NETWORK_PRIVATE_KEY", "test_key");
    std::env::set_var("SKIP_TIME_VALIDITY_CHECK", "true");

    let config = ProverConfig::from_env();
    assert_eq!(config.private_key, Some("test_key".to_string()));
    assert!(config.skip_time_validity_check);
    assert!(config.has_network_key());

    // Clean up
    std::env::remove_var("NETWORK_PRIVATE_KEY");
    std::env::remove_var("SKIP_TIME_VALIDITY_CHECK");
}

#[test]
fn test_config_fallback_to_sp1_private_key() {
    let _guard = ENV_LOCK.lock().expect("ENV_LOCK poisoned");
    std::env::remove_var("NETWORK_PRIVATE_KEY");
    std::env::set_var("SP1_PRIVATE_KEY", "fallback_key");

    let config = ProverConfig::from_env();
    assert_eq!(config.private_key, Some("fallback_key".to_string()));

    std::env::remove_var("SP1_PRIVATE_KEY");
}
