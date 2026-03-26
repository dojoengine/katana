#![cfg(feature = "cartridge")]

//! Full end-to-end tests for the Cartridge RPC flow with real paymaster and VRF
//! server binaries.
//!
//! Requires `paymaster-service` and `vrf-server` binaries to be installed and
//! available in PATH. Skips gracefully if binaries are not found.
//!
//! Uses instant mining so bootstrap transactions (forwarder deploy, VRF account
//! deploy, etc.) are mined immediately. Assertions check on-chain state and
//! RPC responses rather than pool contents.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cartridge::vrf::server::{
    bootstrap_vrf, get_vrf_account, VrfServer, VrfServerConfig, VRF_SERVER_PORT,
};
use jsonrpsee::core::client::ClientT;
use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
use katana_paymaster::{PaymasterService, PaymasterServiceConfigBuilder};
use katana_primitives::{ContractAddress, Felt};
use katana_utils::node::test_config;
use katana_utils::TestNode;
use serde_json::json;
use starknet::macros::selector;
use starknet::providers::Provider;

mod common;

const PAYMASTER_API_KEY: &str = "paymaster_katana";

// ---------------------------------------------------------------------------
// Mock Cartridge API
// ---------------------------------------------------------------------------

async fn start_mock_cartridge_api(
    responses: HashMap<String, serde_json::Value>,
) -> (url::Url, MockCartridgeApiState) {
    use axum::routing::post;
    use axum::Router;
    use tokio::net::TcpListener;

    let state = MockCartridgeApiState {
        responses: Arc::new(responses),
        received_requests: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/accounts/calldata", post(mock_cartridge_handler))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (url::Url::parse(&format!("http://{addr}")).unwrap(), state)
}

async fn mock_cartridge_handler(
    axum::extract::State(state): axum::extract::State<MockCartridgeApiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    state.received_requests.lock().unwrap().push(body.clone());
    let address = body.get("address").and_then(|v| v.as_str()).unwrap_or("");

    if let Some(response) = state.responses.get(address) {
        axum::Json(response.clone()).into_response()
    } else {
        "Address not found".into_response()
    }
}

#[derive(Clone)]
struct MockCartridgeApiState {
    responses: Arc<HashMap<String, serde_json::Value>>,
    received_requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn controller_calldata_response(address: &str) -> serde_json::Value {
    json!({
        "address": address,
        "username": "testuser",
        "calldata": [
            "0x24a9edbfa7082accfceabf6a92d7160086f346d622f28741bf1c651c412c9ab",
            "0x7465737475736572",
            "0x0",
            "0x2",
            "0x1",
            "0x2"
        ]
    })
}

fn find_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn prefunded_account(
    config: &katana_sequencer_node::config::Config,
    index: usize,
) -> (ContractAddress, Felt) {
    use katana_genesis::allocation::GenesisAccountAlloc;

    let (address, alloc) = config.chain.genesis().accounts().nth(index).expect("account exists");
    let pk = match alloc {
        GenesisAccountAlloc::DevAccount(a) => a.private_key,
        _ => panic!("expected dev account"),
    };
    (*address, pk)
}

/// Check if a binary is available in PATH.
fn binary_available(name: &str) -> bool {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

/// Full e2e test with real paymaster-service and vrf-server binaries.
///
/// Exercises:
/// 1. Controller deployment middleware → deploy tx mined on-chain
/// 2. Already-deployed address → no deployment, Cartridge API not queried
/// 3. Non-controller address → no deployment
/// 4. VRF delegation → VRF server processes request, paymaster relays on-chain
/// 5. VRF error: wrong target → VrfInvalidTarget
/// 6. VRF error: no follow-up call → VrfMissingFollowUpCall
#[tokio::test(flavor = "multi_thread")]
async fn cartridge_e2e_with_real_services() {
    // Skip if binaries are not installed.
    if !binary_available("paymaster-service") {
        eprintln!("SKIPPED: paymaster-service not found in PATH");
        return;
    }
    if !binary_available("vrf-server") {
        eprintln!("SKIPPED: vrf-server not found in PATH");
        return;
    }

    // -- Pre-allocate ports --
    let paymaster_port = find_free_port();
    let paymaster_url = url::Url::parse(&format!("http://127.0.0.1:{paymaster_port}")).unwrap();

    // -- Setup mock Cartridge API --
    let controller_address = "0xdead";
    let cartridge_responses = HashMap::from_iter([(
        controller_address.to_string(),
        controller_calldata_response(controller_address),
    )]);
    let (cartridge_api_url, mock_api_state) = start_mock_cartridge_api(cartridge_responses).await;

    // -- Build node config (instant mining) --
    let mut config = test_config();
    config.dev.fee = false;

    // Add controller + VRF + forwarder classes to genesis.
    {
        let chain = Arc::make_mut(&mut config.chain);
        if let katana_chain_spec::ChainSpec::Dev(ref mut spec) = chain {
            katana_slot_controller::add_controller_classes(&mut spec.genesis);
            katana_slot_controller::add_vrf_provider_class(&mut spec.genesis);
            katana_slot_controller::add_avnu_forwarder_class(&mut spec.genesis);
        }
    }

    let (deployer_address, deployer_pk) = prefunded_account(&config, 0);

    // Derive VRF account info (deterministic from hardcoded secret key).
    let vrf_creds = get_vrf_account().expect("derive VRF account");
    let vrf_account_address = vrf_creds.account_address;

    // Configure paymaster + cartridge + VRF.
    {
        use katana_sequencer_node::config::paymaster::{
            CartridgeApiConfig, PaymasterConfig, VrfConfig,
        };

        let vrf_url = url::Url::parse(&format!("http://127.0.0.1:{VRF_SERVER_PORT}")).unwrap();

        config.paymaster = Some(PaymasterConfig {
            url: paymaster_url.clone(),
            api_key: Some(PAYMASTER_API_KEY.to_string()),
            cartridge_api: Some(CartridgeApiConfig {
                cartridge_api_url,
                controller_deployer_address: deployer_address,
                controller_deployer_private_key: deployer_pk,
                vrf: Some(VrfConfig { url: vrf_url, vrf_account: vrf_account_address }),
            }),
        });
    }

    // Grab accounts before moving config into the node.
    let (relayer_addr, relayer_pk) = prefunded_account(&config, 0);
    let (gas_tank_addr, gas_tank_pk) = prefunded_account(&config, 1);
    let (estimate_addr, estimate_pk) = prefunded_account(&config, 2);

    // -- Start node --
    let node = TestNode::new_with_config(config).await;
    let rpc_addr = *node.rpc_addr();
    let client = node.rpc_http_client();
    let provider = node.starknet_provider();

    let paymaster_config = PaymasterServiceConfigBuilder::new()
        .rpc(rpc_addr)
        .port(paymaster_port)
        .api_key(PAYMASTER_API_KEY)
        .relayer(relayer_addr, relayer_pk)
        .gas_tank(gas_tank_addr, gas_tank_pk)
        .estimate_account(estimate_addr, estimate_pk)
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .build()
        .await
        .expect("paymaster config");

    let mut paymaster = PaymasterService::new(paymaster_config);
    paymaster.bootstrap().await.expect("paymaster bootstrap");
    let mut _paymaster_process = paymaster.start().await.expect("paymaster start");

    // -- Bootstrap VRF --
    let rpc_url = url::Url::parse(&format!("http://{rpc_addr}")).unwrap();
    let vrf_result =
        bootstrap_vrf(rpc_url, deployer_address, deployer_pk).await.expect("VRF bootstrap");

    let vrf_server = VrfServer::new(VrfServerConfig {
        secret_key: vrf_result.secret_key,
        vrf_account_address: vrf_result.vrf_account_address,
        vrf_private_key: vrf_result.vrf_account_private_key,
    });
    let mut _vrf_process = vrf_server.start().await.expect("VRF server start");

    // -----------------------------------------------------------------------
    // Case 1: Controller deployment — undeployed controller gets deployed
    // -----------------------------------------------------------------------
    {
        // Verify controller is NOT deployed yet.
        let class_hash = provider
            .get_class_hash_at(
                starknet::core::types::BlockId::Tag(starknet::core::types::BlockTag::PreConfirmed),
                Felt::from_hex_unchecked(controller_address),
            )
            .await;
        assert!(class_hash.is_err(), "controller should not be deployed yet");

        let params = make_execute_outside_params(controller_address);
        let response: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("execute outside should succeed");

        // Response should contain a transaction hash from the paymaster.
        assert!(
            response.get("transaction_hash").is_some(),
            "response should contain transaction_hash: {response:?}"
        );

        // Cartridge API should have been queried.
        let api_requests = mock_api_state.received_requests.lock().unwrap();
        assert!(!api_requests.is_empty(), "Cartridge API should have been queried");

        // Controller should now be deployed (deploy tx was mined).
        // Give it a moment for the block to be produced.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let class_hash = provider
            .get_class_hash_at(
                starknet::core::types::BlockId::Tag(starknet::core::types::BlockTag::PreConfirmed),
                Felt::from_hex_unchecked(controller_address),
            )
            .await;
        assert!(class_hash.is_ok(), "controller should be deployed after execute_outside");
    }

    // -----------------------------------------------------------------------
    // Case 2: Already-deployed address → no deployment
    // -----------------------------------------------------------------------
    {
        let prev_count = mock_api_state.received_requests.lock().unwrap().len();
        let genesis_addr = format!("{:#x}", Felt::from(deployer_address));

        let params = make_execute_outside_params(&genesis_addr);
        let _: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("should succeed for deployed address");

        // Cartridge API should NOT have been queried again.
        let api_requests = mock_api_state.received_requests.lock().unwrap();
        assert_eq!(
            api_requests.len(),
            prev_count,
            "Cartridge API should not be queried for already-deployed accounts"
        );
    }

    // -----------------------------------------------------------------------
    // Case 3: Non-controller address → no deployment
    // -----------------------------------------------------------------------
    {
        let prev_count = mock_api_state.received_requests.lock().unwrap().len();
        let non_controller = "0xbeef";

        let params = make_execute_outside_params(non_controller);
        let _: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("should succeed for non-controller");

        // Cartridge API WAS queried (undeployed address).
        let api_requests = mock_api_state.received_requests.lock().unwrap();
        assert_eq!(api_requests.len(), prev_count + 1, "Cartridge API should be queried");

        // But address is still not deployed (not a controller).
        let class_hash = provider
            .get_class_hash_at(
                starknet::core::types::BlockId::Tag(starknet::core::types::BlockTag::PreConfirmed),
                Felt::from_hex_unchecked(non_controller),
            )
            .await;
        assert!(class_hash.is_err(), "non-controller should not be deployed");
    }

    // -----------------------------------------------------------------------
    // Case 4: VRF flow — request_random triggers VRF delegation
    // -----------------------------------------------------------------------
    {
        let genesis_addr = format!("{:#x}", Felt::from(deployer_address));
        let vrf_addr = format!("{:#x}", Felt::from(vrf_account_address));

        let params = make_vrf_execute_outside_params(&genesis_addr, &vrf_addr);
        let response: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("VRF execute outside should succeed");

        assert!(
            response.get("transaction_hash").is_some(),
            "VRF response should contain transaction_hash: {response:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Case 5: VRF wrong target → error
    // -----------------------------------------------------------------------
    {
        let genesis_addr = format!("{:#x}", Felt::from(deployer_address));

        let params = vec![
            json!(genesis_addr),
            json!({
                "caller": "0x414e595f43414c4c4552",
                "nonce": "0x5",
                "execute_after": "0x0",
                "execute_before": "0xffffffffffffffff",
                "calls": [
                    {
                        "to": "0xdead",
                        "selector": format!("{:#x}", selector!("request_random")),
                        "calldata": ["0x1"]
                    },
                    {
                        "to": "0xaaa",
                        "selector": "0xbbb",
                        "calldata": []
                    }
                ]
            }),
            json!(["0x0", "0x0"]),
            json!(null),
        ];

        let result: Result<serde_json::Value, _> =
            client.request("cartridge_addExecuteOutsideTransaction", params).await;
        assert!(result.is_err(), "should fail with VrfInvalidTarget");
    }

    // -----------------------------------------------------------------------
    // Case 6: VRF no follow-up call → error
    // -----------------------------------------------------------------------
    {
        let genesis_addr = format!("{:#x}", Felt::from(deployer_address));
        let vrf_addr = format!("{:#x}", Felt::from(vrf_account_address));

        let params = vec![
            json!(genesis_addr),
            json!({
                "caller": "0x414e595f43414c4c4552",
                "nonce": "0x6",
                "execute_after": "0x0",
                "execute_before": "0xffffffffffffffff",
                "calls": [{
                    "to": vrf_addr,
                    "selector": format!("{:#x}", selector!("request_random")),
                    "calldata": ["0x1"]
                }]
            }),
            json!(["0x0", "0x0"]),
            json!(null),
        ];

        let result: Result<serde_json::Value, _> =
            client.request("cartridge_addExecuteOutsideTransaction", params).await;
        assert!(result.is_err(), "should fail with VrfMissingFollowUpCall");
    }

    // -- Cleanup --
    let _ = _paymaster_process.shutdown().await;
    let _ = _vrf_process.shutdown().await;
}

// ---------------------------------------------------------------------------
// Param builders
// ---------------------------------------------------------------------------

fn make_execute_outside_params(address: &str) -> Vec<serde_json::Value> {
    vec![
        json!(address),
        json!({
            "caller": "0x414e595f43414c4c4552",
            "nonce": "0x1",
            "execute_after": "0x0",
            "execute_before": "0xffffffffffffffff",
            "calls": [{
                "to": "0x1",
                "selector": "0x2",
                "calldata": ["0x3"]
            }]
        }),
        json!(["0x0", "0x0"]),
        json!(null),
    ]
}

fn make_vrf_execute_outside_params(
    player_address: &str,
    vrf_account_address: &str,
) -> Vec<serde_json::Value> {
    vec![
        json!(player_address),
        json!({
            "caller": "0x414e595f43414c4c4552",
            "nonce": "0x3",
            "execute_after": "0x0",
            "execute_before": "0xffffffffffffffff",
            "calls": [
                {
                    "to": vrf_account_address,
                    "selector": format!("{:#x}", selector!("request_random")),
                    "calldata": ["0x1", "0x2"]
                },
                {
                    "to": "0xaaa",
                    "selector": "0xbbb",
                    "calldata": []
                }
            ]
        }),
        json!(["0x0", "0x0"]),
        json!(null),
    ]
}
