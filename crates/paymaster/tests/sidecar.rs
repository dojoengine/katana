//! Integration tests for paymaster sidecar spawning.
//!
//! These tests verify that the paymaster sidecar can be spawned correctly
//! without requiring a running katana node.
//!
//! Prerequisites:
//! - paymaster-service binary must be installed: ``` git clone https://github.com/avnu-labs/paymaster
//!   cd paymaster cargo build --release --bin paymaster-service ```

use std::path::PathBuf;
use std::time::Duration;

use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
use katana_paymaster::{wait_for_paymaster_ready, PaymasterConfigBuilder, PaymasterSidecar};
use katana_primitives::chain::ChainId;
use katana_primitives::{address, felt};
use url::Url;

/// Test that the sidecar process can be spawned and responds to health checks.
///
/// This test requires the paymaster-service binary to be installed.
/// Run with: `cargo nextest run -p katana-paymaster test_sidecar_spawn_and_health_check --ignored`
#[tokio::test]
#[ignore = "requires paymaster-service binary to be installed"]
async fn test_sidecar_spawn_and_health_check() {
    let port = 3030;
    let api_key = "test-api-key".to_string();

    // Build the config using the builder pattern (unchecked since we don't have a real node)
    let config = PaymasterConfigBuilder::new()
        .rpc_url(Url::parse("http://127.0.0.1:5050").unwrap())
        .port(port)
        .api_key(api_key.clone())
        .relayer(address!("0x1"), felt!("0x1"))
        .gas_tank(address!("0x2"), felt!("0x2"))
        .estimate_account(address!("0x3"), felt!("0x3"))
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .build_unchecked()
        .expect("config should build");

    // Create sidecar with forwarder and chain_id set directly (skip bootstrap)
    let sidecar =
        PaymasterSidecar::new(config).forwarder(address!("0x4")).chain_id(ChainId::SEPOLIA);

    // Spawn the sidecar
    let mut process = sidecar.start().await.expect("failed to spawn paymaster sidecar");

    // Verify it responds to health checks
    let health_url = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let result =
        wait_for_paymaster_ready(&health_url, Some(&api_key), Duration::from_secs(10)).await;

    // Clean up
    process.shutdown().await.ok();

    // Assert health check passed
    result.expect("sidecar should respond to health check");
}

/// Test that spawning fails gracefully when binary is not found.
#[tokio::test]
async fn test_sidecar_spawn_binary_not_found() {
    let port = 3031;
    let api_key = "test-api-key".to_string();

    // Build the config with a nonexistent binary path
    let config = PaymasterConfigBuilder::new()
        .rpc_url(Url::parse("http://127.0.0.1:5050").unwrap())
        .port(port)
        .api_key(api_key)
        .relayer(address!("0x1"), felt!("0x1"))
        .gas_tank(address!("0x2"), felt!("0x2"))
        .estimate_account(address!("0x3"), felt!("0x3"))
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .program_path(PathBuf::from("/nonexistent/path/to/paymaster-service"))
        .build_unchecked()
        .expect("config should build");

    let sidecar =
        PaymasterSidecar::new(config).forwarder(address!("0x4")).chain_id(ChainId::SEPOLIA);

    let result = sidecar.start().await;
    assert!(result.is_err(), "should fail when binary not found");

    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not found"),
        "error should mention binary not found, got: {err}"
    );
}

/// Test that the builder fails when required fields are missing.
#[tokio::test]
async fn test_builder_missing_required_fields() {
    // Missing rpc_url
    let result = PaymasterConfigBuilder::new()
        .port(3030)
        .api_key("key".to_string())
        .relayer(address!("0x1"), felt!("0x1"))
        .gas_tank(address!("0x2"), felt!("0x2"))
        .estimate_account(address!("0x3"), felt!("0x3"))
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .build_unchecked();

    assert!(result.is_err(), "should fail when rpc_url is missing");
    assert!(result.unwrap_err().to_string().contains("rpc_url"), "error should mention rpc_url");

    // Missing api_key
    let result = PaymasterConfigBuilder::new()
        .rpc_url(Url::parse("http://127.0.0.1:5050").unwrap())
        .port(3030)
        // Missing api_key
        .relayer(address!("0x1"), felt!("0x1"))
        .gas_tank(address!("0x2"), felt!("0x2"))
        .estimate_account(address!("0x3"), felt!("0x3"))
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .build_unchecked();

    assert!(result.is_err(), "should fail when api_key is missing");
    assert!(result.unwrap_err().to_string().contains("api_key"), "error should mention api_key");

    // Missing relayer
    let result = PaymasterConfigBuilder::new()
        .rpc_url(Url::parse("http://127.0.0.1:5050").unwrap())
        .port(3030)
        .api_key("key".to_string())
        // Missing relayer
        .gas_tank(address!("0x2"), felt!("0x2"))
        .estimate_account(address!("0x3"), felt!("0x3"))
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .build_unchecked();

    assert!(result.is_err(), "should fail when relayer is missing");
    assert!(
        result.unwrap_err().to_string().contains("relayer_address"),
        "error should mention relayer_address"
    );

    // Missing tokens
    let result = PaymasterConfigBuilder::new()
        .rpc_url(Url::parse("http://127.0.0.1:5050").unwrap())
        .port(3030)
        .api_key("key".to_string())
        .relayer(address!("0x1"), felt!("0x1"))
        .gas_tank(address!("0x2"), felt!("0x2"))
        .estimate_account(address!("0x3"), felt!("0x3"))
        // Missing tokens
        .build_unchecked();

    assert!(result.is_err(), "should fail when tokens are missing");
    assert!(
        result.unwrap_err().to_string().contains("eth_token_address"),
        "error should mention eth_token_address"
    );
}

/// Test that the wait_for_paymaster_ready function times out correctly.
#[tokio::test]
async fn test_wait_for_paymaster_ready_timeout() {
    // Try to connect to a non-existent service
    let url = Url::parse("http://127.0.0.1:39999").unwrap();
    let result = wait_for_paymaster_ready(&url, None, Duration::from_millis(500)).await;

    assert!(result.is_err(), "should timeout when service not available");

    let err = result.unwrap_err();
    assert!(err.to_string().contains("timeout"), "error should mention timeout, got: {err}");
}
