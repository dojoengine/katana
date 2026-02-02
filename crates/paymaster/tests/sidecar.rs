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
use katana_paymaster::{start_paymaster_sidecar, wait_for_paymaster_ready, PaymasterSidecarConfig};
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
    let config = PaymasterSidecarConfig {
        program_path: None, // Use PATH lookup
        port: 3030,
        api_key: "test-api-key".to_string(),
        price_api_key: None,

        // Mock addresses (the sidecar doesn't validate these on startup)
        relayer_address: address!("0x1"),
        relayer_private_key: felt!("0x1"),
        gas_tank_address: address!("0x2"),
        gas_tank_private_key: felt!("0x2"),
        estimate_account_address: address!("0x3"),
        estimate_account_private_key: felt!("0x3"),
        forwarder_address: address!("0x4"),

        chain_id: ChainId::SEPOLIA,
        rpc_url: Url::parse("http://127.0.0.1:5050").unwrap(), // Mock RPC URL

        eth_token_address: DEFAULT_ETH_FEE_TOKEN_ADDRESS,
        strk_token_address: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
    };

    // Spawn the sidecar
    let mut child =
        start_paymaster_sidecar(&config).await.expect("failed to spawn paymaster sidecar");

    // Verify it responds to health checks
    let health_url = Url::parse(&format!("http://127.0.0.1:{}", config.port)).unwrap();
    let result =
        wait_for_paymaster_ready(&health_url, Some(&config.api_key), Duration::from_secs(10)).await;

    // Clean up
    child.kill().await.ok();

    // Assert health check passed
    result.expect("sidecar should respond to health check");
}

/// Test that spawning fails gracefully when binary is not found.
#[tokio::test]
async fn test_sidecar_spawn_binary_not_found() {
    let config = PaymasterSidecarConfig {
        program_path: Some(PathBuf::from("/nonexistent/path/to/paymaster-service")),
        port: 3031,
        api_key: "test-api-key".to_string(),
        price_api_key: None,

        // Mock addresses
        relayer_address: address!("0x1"),
        relayer_private_key: felt!("0x1"),
        gas_tank_address: address!("0x2"),
        gas_tank_private_key: felt!("0x2"),
        estimate_account_address: address!("0x3"),
        estimate_account_private_key: felt!("0x3"),
        forwarder_address: address!("0x4"),

        chain_id: ChainId::SEPOLIA,
        rpc_url: Url::parse("http://127.0.0.1:5050").unwrap(),

        eth_token_address: DEFAULT_ETH_FEE_TOKEN_ADDRESS,
        strk_token_address: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
    };

    let result = start_paymaster_sidecar(&config).await;
    assert!(result.is_err(), "should fail when binary not found");

    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not found"),
        "error should mention binary not found, got: {err}"
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
