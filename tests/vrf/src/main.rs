use std::sync::Arc;

use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;
use cairo_lang_starknet_classes::contract_class::ContractClass;
use cartridge::vrf::server::{
    bootstrap_vrf, get_vrf_account, VrfServer, VrfServerConfig, VRF_SERVER_PORT,
};
use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
use katana_paymaster::{PaymasterService, PaymasterServiceConfigBuilder};
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::class::CompiledClass;
use katana_primitives::execution::Call;
use katana_primitives::{address, felt, ContractAddress, Felt};
use katana_rpc_api::cartridge::CartridgeApiClient;
use katana_rpc_types::{OutsideExecution, OutsideExecutionV2};
use katana_sequencer_node::config::paymaster::{CartridgeApiConfig, PaymasterConfig, VrfConfig};
use katana_utils::node::{test_config, TestNode};
use starknet::accounts::Account;
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::core::types::contract::SierraClass;
use starknet::core::utils::get_contract_address;
use starknet::macros::selector;
use tokio::net::TcpListener;
use url::Url;

const SIERRA_CLASS: &str = include_str!("../build/vrng_test_Simple.contract_class.json");

/// ANY_CALLER constant for outside execution.
const ANY_CALLER: ContractAddress = address!("0x414e595f43414c4c4552");

#[tokio::main]
async fn main() {
    // --- A. Pre-allocate ports and compute addresses ---

    let paymaster_port = find_free_port();
    let paymaster_url =
        Url::parse(&format!("http://127.0.0.1:{paymaster_port}")).expect("valid url");
    let vrf_url =
        Url::parse(&format!("http://127.0.0.1:{VRF_SERVER_PORT}")).expect("valid url");

    let vrf_cred = get_vrf_account().expect("failed to derive VRF account");
    let vrf_account_address = vrf_cred.account_address;

    let cartridge_api_url = start_mock_cartridge_api().await;

    // --- B. Build node config with paymaster + VRF ---

    let mut config = test_config();

    // Pre-assign the RPC port so the VRF server gets the correct RPC URL
    // (the node builds VrfServiceConfig.rpc_url from config.rpc.socket_addr()).
    let rpc_port = find_free_port();
    config.rpc.port = rpc_port;

    let (deployer_address, deployer_account) =
        config.chain.genesis().accounts().next().expect("must have genesis accounts");
    let deployer_private_key = deployer_account.private_key().expect("must have private key");

    config.paymaster = Some(PaymasterConfig {
        url: paymaster_url.clone(),
        api_key: Some("paymaster_katana".into()),
        cartridge_api: Some(CartridgeApiConfig {
            cartridge_api_url,
            controller_deployer_address: *deployer_address,
            controller_deployer_private_key: deployer_private_key,
            vrf: Some(VrfConfig { url: vrf_url, vrf_account: vrf_account_address }),
        }),
    });

    // --- C. Start the node, bootstrap + start sidecars ---

    let node = TestNode::new_with_config(config.clone()).await;
    let rpc_addr = *node.rpc_addr();
    let rpc_url = Url::parse(&format!("http://127.0.0.1:{}", rpc_addr.port())).expect("valid url");

    println!("Node started at {rpc_url}");

    // Bootstrap and start paymaster
    let (account_0_addr, account_0_pk) = genesis_account(&config, 0);
    let (account_1_addr, account_1_pk) = genesis_account(&config, 1);
    let (account_2_addr, account_2_pk) = genesis_account(&config, 2);

    let paymaster_cfg = PaymasterServiceConfigBuilder::new()
        .rpc(rpc_addr)
        .port(paymaster_port)
        .api_key("paymaster_katana")
        .relayer(account_0_addr, account_0_pk)
        .gas_tank(account_1_addr, account_1_pk)
        .estimate_account(account_2_addr, account_2_pk)
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS)
        .build()
        .await
        .expect("failed to build paymaster config");

    let mut paymaster = PaymasterService::new(paymaster_cfg);
    let forwarder_address = paymaster.bootstrap().await.expect("failed to bootstrap paymaster");
    let mut paymaster_process = paymaster.start().await.expect("failed to start paymaster");

    println!("Paymaster started on port {paymaster_port}");
    println!("Relayer (account[0]): {account_0_addr}");
    println!("Gas tank (account[1]): {account_1_addr}");
    println!("Estimate (account[2]): {account_2_addr}");
    println!("Forwarder at {forwarder_address}");

    // Bootstrap and start VRF
    let vrf_result = bootstrap_vrf(rpc_url.clone(), account_0_addr, account_0_pk)
        .await
        .expect("failed to bootstrap VRF");

    println!("VRF bootstrapped: account={:#x}", Felt::from(vrf_result.vrf_account_address));

    // Whitelist the VRF account and estimate account on the AVNU forwarder.
    // The VRF account is the user_address for VRF txs; the estimate account is
    // used by the paymaster for fee estimation (simulate_transaction).
    for addr in [vrf_result.vrf_account_address, account_2_addr] {
        whitelist_on_forwarder(&node, forwarder_address, addr, account_0_addr, account_0_pk)
            .await;
    }

    let mut vrf_process = VrfServer::new(VrfServerConfig {
        secret_key: vrf_result.secret_key,
        vrf_account_address: vrf_result.vrf_account_address,
        vrf_private_key: vrf_result.vrf_account_private_key,
    })
    .start()
    .await
    .expect("failed to start VRF server");

    println!("VRF server started on port {VRF_SERVER_PORT}");

    // --- D. Declare and deploy Simple contract ---

    let simple_contract_address =
        declare_and_deploy_simple(&node, vrf_result.vrf_account_address).await;

    println!("Simple contract deployed at {simple_contract_address:#x}");

    // --- E. Submit VRF transactions ---

    let player_address: ContractAddress = node.account().address().into();
    let player_signature = vec![felt!("0x0"), felt!("0x0")];

    // Test roll_dice_with_nonce
    {
        let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
            caller: ANY_CALLER,
            nonce: felt!("0x1"),
            execute_after: 0,
            execute_before: 0xffffffffffffffff,
            calls: vec![
                // request_random(caller=simple_contract, source=Nonce(player))
                Call {
                    contract_address: vrf_account_address,
                    entry_point_selector: selector!("request_random"),
                    calldata: vec![
                        simple_contract_address.into(), // caller
                        Felt::ZERO,                     // Source::Nonce variant
                        player_address.into(),          // Nonce(player_address)
                    ],
                },
                // roll_dice_with_nonce()
                Call {
                    contract_address: simple_contract_address.into(),
                    entry_point_selector: selector!("roll_dice_with_nonce"),
                    calldata: vec![],
                },
            ],
        });

        let res = node
            .rpc_http_client()
            .add_execute_outside_transaction(
                player_address,
                outside_execution,
                player_signature.clone(),
                None,
            )
            .await
            .expect("roll_dice_with_nonce outside execution failed");

        println!("roll_dice_with_nonce tx: {:#x}", res.transaction_hash);
    }

    // Test roll_dice_with_salt
    {
        let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
            caller: ANY_CALLER,
            nonce: felt!("0x2"),
            execute_after: 0,
            execute_before: 0xffffffffffffffff,
            calls: vec![
                // request_random(caller=simple_contract, source=Salt(42))
                Call {
                    contract_address: vrf_account_address,
                    entry_point_selector: selector!("request_random"),
                    calldata: vec![
                        simple_contract_address.into(), // caller
                        Felt::ONE,                      // Source::Salt variant
                        Felt::from(42u64),              // Salt(42)
                    ],
                },
                // roll_dice_with_salt()
                Call {
                    contract_address: simple_contract_address.into(),
                    entry_point_selector: selector!("roll_dice_with_salt"),
                    calldata: vec![],
                },
            ],
        });

        let res = node
            .rpc_http_client()
            .add_execute_outside_transaction(
                player_address,
                outside_execution,
                player_signature,
                None,
            )
            .await
            .expect("roll_dice_with_salt outside execution failed");

        println!("roll_dice_with_salt tx: {:#x}", res.transaction_hash);
    }

    // --- F. Verify results ---

    let provider = node.starknet_rpc_client();
    let response = provider
        .call(
            katana_primitives::execution::Call {
                contract_address: simple_contract_address.into(),
                entry_point_selector: selector!("get_value"),
                calldata: vec![],
            },
            BlockIdOrTag::PreConfirmed,
        )
        .await
        .expect("get_value call failed");

    assert!(!response.result.is_empty(), "get_value returned empty result");
    assert_ne!(response.result[0], Felt::ZERO, "VRF random value should be non-zero");

    println!("VRF value stored in contract: {:#x}", response.result[0]);
    println!("All assertions passed.");

    // --- G. Cleanup ---

    vrf_process.shutdown().await.expect("failed to shutdown VRF");
    paymaster_process.shutdown().await.expect("failed to shutdown paymaster");
}

// =============================================================================
// Helper functions
// =============================================================================

/// Declares and deploys the Simple contract with the VRF provider address as constructor arg.
async fn declare_and_deploy_simple(node: &TestNode, vrf_provider: ContractAddress) -> Felt {
    let account = node.account();
    let provider = node.starknet_rpc_client();

    // Declare
    let sierra: SierraClass = serde_json::from_str(SIERRA_CLASS).expect("invalid sierra class");
    let class_hash = sierra.class_hash().expect("failed to compute class hash");
    let flattened = sierra.flatten().expect("failed to flatten sierra class");

    let contract: ContractClass =
        serde_json::from_str(SIERRA_CLASS).expect("invalid contract class");
    let casm = CasmContractClass::from_contract_class(contract, true, usize::MAX)
        .expect("failed to compile to CASM");
    let casm_json = serde_json::to_string(&casm).expect("failed to serialize CASM");
    let compiled: CompiledClass =
        serde_json::from_str(&casm_json).expect("invalid compiled class");
    let compiled_class_hash = compiled.class_hash().expect("failed to compute compiled class hash");

    let res = account
        .declare_v3(Arc::new(flattened), compiled_class_hash)
        .send()
        .await
        .expect("declare failed");

    katana_utils::TxWaiter::new(res.transaction_hash, &provider)
        .await
        .expect("declare tx failed");

    // Deploy with VRF provider address as constructor arg
    let salt = Felt::ZERO;
    let constructor_calldata = vec![vrf_provider.into()];
    let factory = ContractFactory::new_with_udc(class_hash, &account, UdcSelector::Legacy);
    let deployment = factory.deploy_v3(constructor_calldata.clone(), salt, false);

    let address = get_contract_address(salt, class_hash, &constructor_calldata, Felt::ZERO);

    let res = deployment.send().await.expect("deploy failed");

    katana_utils::TxWaiter::new(res.transaction_hash, &provider)
        .await
        .expect("deploy tx failed");

    address
}

/// Starts a minimal mock Cartridge Controller API that always returns "Address not found".
async fn start_mock_cartridge_api() -> Url {
    async fn handler(axum::Json(_body): axum::Json<serde_json::Value>) -> axum::response::Response {
        "Address not found".into_response()
    }

    let app = Router::new().route("/accounts/calldata", post(handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    Url::parse(&format!("http://{addr}")).unwrap()
}

/// Extracts genesis account address and private key by index.
fn genesis_account(
    config: &katana_sequencer_node::config::Config,
    index: usize,
) -> (ContractAddress, Felt) {
    let (address, account) = config
        .chain
        .genesis()
        .accounts()
        .nth(index)
        .expect("not enough genesis accounts");
    let private_key = account.private_key().expect("missing private key");
    (*address, private_key)
}

/// Whitelists an address on the AVNU forwarder contract.
async fn whitelist_on_forwarder(
    node: &TestNode,
    forwarder: ContractAddress,
    address_to_whitelist: ContractAddress,
    signer_address: ContractAddress,
    signer_pk: Felt,
) {
    use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
    use starknet::core::types::BlockTag;
    use starknet::signers::{LocalWallet, SigningKey};

    let provider = node.starknet_provider();
    let chain_id = node.backend().chain_spec.id();
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(signer_pk));

    let mut account = SingleOwnerAccount::new(
        provider,
        signer,
        signer_address.into(),
        chain_id.into(),
        ExecutionEncoding::New,
    );
    account.set_block_id(starknet::core::types::BlockId::Tag(BlockTag::PreConfirmed));

    let call = starknet::core::types::Call {
        to: forwarder.into(),
        selector: selector!("set_whitelisted_address"),
        calldata: vec![address_to_whitelist.into(), Felt::ONE],
    };

    let res = account.execute_v3(vec![call]).send().await.expect("whitelist tx failed");

    katana_utils::TxWaiter::new(res.transaction_hash, &node.starknet_rpc_client())
        .await
        .expect("whitelist tx not confirmed");

    println!("Whitelisted VRF account {address_to_whitelist} on forwarder");
}

fn find_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
