mod utils;

use cainome::rs::abigen;
use cartridge::vrf::server::get_default_vrf_account;
use katana_cli::sidecar;
use katana_primitives::execution::Call;
use katana_primitives::utils::get_contract_address;
use katana_primitives::{address, felt, ContractAddress, Felt};
use katana_rpc_api::cartridge::CartridgeApiClient;
use katana_rpc_types::{OutsideExecution, OutsideExecutionV2};
use katana_sequencer_node::config::paymaster::{CartridgeApiConfig, PaymasterConfig, VrfConfig};
use katana_utils::find_free_port;
use katana_utils::node::{test_config, TestNode};
use starknet::accounts::Account;
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, SigningKey};
use url::Url;

use crate::utils::start_mock_cartridge_api;

abigen!(SimpleVrfApp,
[
    {
        "type": "function",
        "name": "vrf_set_with_nonce",
        "inputs": [],
        "outputs": [],
        "state_mutability": "external"
    },
    {
        "type": "function",
        "name": "vrf_set_with_salt",
        "inputs": [],
        "outputs": [],
        "state_mutability": "external"
    },
    {
        "type": "function",
        "name": "get",
        "inputs": [],
        "outputs": [{ "type": "core::felt252" }],
        "state_mutability": "view"
    },
    {
        "type": "function",
        "name": "set",
        "inputs": [{ "name": "value", "type": "core::felt252" }],
        "outputs": [],
        "state_mutability": "external"
    }
]);

/// ANY_CALLER constant for outside execution.
const ANY_CALLER: ContractAddress = address!("0x414e595f43414c4c4552");

#[tokio::main]
async fn main() {
    // --- A. Pre-allocate ports and compute addresses ---

    let paymaster_port = find_free_port();
    let paymaster_url =
        Url::parse(&format!("http://127.0.0.1:{paymaster_port}")).expect("valid url");
    let vrf_url = Url::parse(&format!("http://127.0.0.1:3000")).expect("valid url");

    let vrf_cred = get_default_vrf_account().expect("failed to derive VRF account");
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
        api_key: Some(sidecar::DEFAULT_PAYMASTER_API_KEY.into()),
        cartridge_api: Some(CartridgeApiConfig {
            cartridge_api_url,
            controller_deployer_address: *deployer_address,
            controller_deployer_private_key: deployer_private_key,
            vrf: Some(VrfConfig { url: vrf_url.clone(), vrf_account: vrf_account_address }),
        }),
    });

    // --- C. Start the node, bootstrap + start sidecars ---

    let node = TestNode::new_with_config(config.clone()).await;
    let rpc_addr = *node.rpc_addr();
    let rpc_url = Url::parse(&format!("http://127.0.0.1:{}", rpc_addr.port())).expect("valid url");

    println!("Node started at {rpc_url}");

    // Bootstrap and start paymaster using the sidecar helper
    let paymaster_bin = utils::find_in_path("paymaster-service").expect(
        "paymaster-service binary not found in PATH. Build it from the rev in \
         sidecar-versions.toml",
    );
    let paymaster =
        sidecar::bootstrap_paymaster(paymaster_bin, paymaster_url, rpc_addr, &config.chain)
            .await
            .expect("failed to bootstrap paymaster");

    let mut paymaster_process = paymaster.start().await.expect("failed to start paymaster");

    println!("Paymaster started on port {paymaster_port}");

    // Bootstrap and start VRF using the sidecar helper
    let vrf_bin = utils::find_in_path("vrf-server").expect(
        "vrf-server binary not found in PATH. Build it from the rev in sidecar-versions.toml",
    );

    let vrf_server = sidecar::bootstrap_vrf(vrf_bin, vrf_url, rpc_addr, &config.chain)
        .await
        .expect("failed to bootstrap VRF");

    let vrf_result = get_default_vrf_account().expect("failed to derive VRF account");
    println!("VRF bootstrapped: account={:#x}", Felt::from(vrf_result.account_address));

    // Compute the AVNU forwarder address (deterministic from salt + constructor args).
    let (account_0_addr, account_0_pk) = genesis_account(&config, 0);
    let account_1_addr = genesis_account(&config, 1).0;
    let account_2_addr = genesis_account(&config, 2).0;
    let forwarder_address: ContractAddress = katana_primitives::utils::get_contract_address(
        Felt::from(0x12345u64), // FORWARDER_SALT
        katana_contracts::avnu::AvnuForwarder::HASH,
        &[account_0_addr.into(), account_1_addr.into()],
        ContractAddress::ZERO,
    )
    .into();

    // Whitelist the VRF account and estimate account on the AVNU forwarder.
    // The VRF account is the user_address for VRF txs; the estimate account is
    // used by the paymaster for fee estimation (simulate_transaction).
    for addr in [vrf_result.account_address, account_2_addr] {
        whitelist_on_forwarder(&node, forwarder_address, addr, account_0_addr, account_0_pk).await;
    }

    let mut vrf_process = vrf_server.start().await.expect("failed to start VRF server");

    println!("VRF server started on port 3000");

    // --- D. Deploy a player account with SRC9 support and the Simple contract ---

    // The VRF flow calls execute_from_outside_v2 on the player's account, so it must
    // support SRC9. Genesis accounts don't, so deploy a CartridgeVrfAccount as the player.
    let player_pk = Felt::from(0xBEEFu64);
    let player = deploy_player_account(&node, player_pk, account_0_addr, account_0_pk).await;

    println!("Player account deployed at {player}");

    let simple_contract_address = utils::bootstrap_app(&node, vrf_account_address).await;

    println!("Simple contract deployed at {simple_contract_address:#x}");

    // --- VERIFY INITIAL CONTRACT STATE ---

    let provider = node.starknet_provider();
    let vrf_app_contract = SimpleVrfAppReader::new(simple_contract_address, provider);

    let value = vrf_app_contract.get().call().await.expect("get_value call failed");
    assert_eq!(value, Felt::ZERO, "VRF random value should be initially zero");

    // --- E. Submit VRF transactions ---

    let player_address: ContractAddress = player;
    let player_signer = LocalWallet::from(SigningKey::from_secret_scalar(player_pk));
    let chain_id = node.backend().chain_spec.id().id();

    // Test set_with_nonce
    {
        let outside_execution = OutsideExecutionV2 {
            caller: ANY_CALLER,
            nonce: felt!("0x1"),
            execute_after: 0,
            execute_before: 0xffffffffffffffff,
            calls: vec![
                Call {
                    contract_address: vrf_account_address,
                    entry_point_selector: selector!("request_random"),
                    calldata: vec![
                        simple_contract_address.into(),
                        Felt::ZERO,
                        player_address.into(),
                    ],
                },
                Call {
                    contract_address: simple_contract_address.into(),
                    entry_point_selector: selector!("vrf_set_with_nonce"),
                    calldata: vec![],
                },
            ],
        };

        let signature = utils::sign_outside_execution_v2(
            &outside_execution,
            chain_id,
            player_address,
            &player_signer,
        )
        .await;

        let res = node
            .rpc_http_client()
            .add_execute_outside_transaction(
                player_address,
                OutsideExecution::V2(outside_execution),
                signature,
                None,
            )
            .await
            .expect("vrf_set_with_nonce outside execution failed");

        println!("vrf_set_with_nonce tx: {:#x}", res.transaction_hash);
    }

    // Test set_with_salt
    {
        let outside_execution = OutsideExecutionV2 {
            caller: ANY_CALLER,
            nonce: felt!("0x2"),
            execute_after: 0,
            execute_before: 0xffffffffffffffff,
            calls: vec![
                Call {
                    contract_address: vrf_account_address,
                    entry_point_selector: selector!("request_random"),
                    calldata: vec![simple_contract_address.into(), Felt::ONE, Felt::from(42u64)],
                },
                Call {
                    contract_address: simple_contract_address.into(),
                    entry_point_selector: selector!("vrf_set_with_salt"),
                    calldata: vec![],
                },
            ],
        };

        let signature = utils::sign_outside_execution_v2(
            &outside_execution,
            chain_id,
            player_address,
            &player_signer,
        )
        .await;

        let res = node
            .rpc_http_client()
            .add_execute_outside_transaction(
                player_address,
                OutsideExecution::V2(outside_execution),
                signature,
                None,
            )
            .await
            .expect("vrf_set_with_salt outside execution failed");

        println!("vrf_set_with_salt tx: {:#x}", res.transaction_hash);
    }

    // --- F. Verify results ---

    let provider = node.starknet_provider();
    let vrf_app_contract = SimpleVrfAppReader::new(simple_contract_address, provider);

    let value = vrf_app_contract.get().call().await.expect("get call failed");
    assert_ne!(value, Felt::ZERO, "VRF random value should be non-zero");

    println!("All assertions passed.");

    // --- G. Cleanup ---

    vrf_process.shutdown().await.expect("failed to shutdown VRF");
    paymaster_process.shutdown().await.expect("failed to shutdown paymaster");
}

// =============================================================================
// Helper functions
// =============================================================================

/// Extracts genesis account address and private key by index.
fn genesis_account(
    config: &katana_sequencer_node::config::Config,
    index: usize,
) -> (ContractAddress, Felt) {
    let (address, account) =
        config.chain.genesis().accounts().nth(index).expect("not enough genesis accounts");
    let private_key = account.private_key().expect("missing private key");
    (*address, private_key)
}

/// Deploys a CartridgeVrfAccount as the player account (supports SRC9/outside execution).
/// Funds it from the given bootstrapper account.
async fn deploy_player_account(
    node: &TestNode,
    player_private_key: Felt,
    bootstrapper_addr: ContractAddress,
    bootstrapper_pk: Felt,
) -> ContractAddress {
    use katana_contracts::vrf::CartridgeVrfAccount;
    use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};

    let provider = node.starknet_provider();
    let chain_id = node.backend().chain_spec.id();
    let rpc_client = node.starknet_rpc_client();

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(bootstrapper_pk));
    let account = SingleOwnerAccount::new(
        provider,
        signer,
        bootstrapper_addr.into(),
        chain_id.into(),
        ExecutionEncoding::New,
    );

    // Deploy using the already-declared CartridgeVrfAccount class
    let player_public_key =
        SigningKey::from_secret_scalar(player_private_key).verifying_key().scalar();
    let salt = Felt::from(0xBEEFu64);
    let constructor_calldata = vec![player_public_key];

    let factory =
        ContractFactory::new_with_udc(CartridgeVrfAccount::HASH, &account, UdcSelector::Legacy);
    let deployment = factory.deploy_v3(constructor_calldata.clone(), salt, false);

    let player_address = get_contract_address(
        salt,
        CartridgeVrfAccount::HASH,
        &constructor_calldata,
        ContractAddress::ZERO,
    );

    let res = deployment.send().await.expect("deploy player account failed");
    katana_utils::TxWaiter::new(res.transaction_hash, &rpc_client)
        .await
        .expect("deploy player tx failed");

    // Fund the player account with STRK
    let amount = Felt::from(1_000_000_000_000_000_000u128);
    let transfer = starknet::core::types::Call {
        to: katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(),
        selector: selector!("transfer"),
        calldata: vec![player_address.into(), amount, Felt::ZERO],
    };
    let res = account.execute_v3(vec![transfer]).send().await.expect("fund player failed");
    katana_utils::TxWaiter::new(res.transaction_hash, &rpc_client)
        .await
        .expect("fund player tx failed");

    player_address.into()
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

    let provider = node.starknet_provider();
    let chain_id = node.backend().chain_spec.id();
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(signer_pk));

    let account = SingleOwnerAccount::new(
        provider,
        signer,
        signer_address.into(),
        chain_id.into(),
        ExecutionEncoding::New,
    );

    let call = starknet::core::types::Call {
        to: forwarder.into(),
        selector: selector!("set_whitelisted_address"),
        calldata: vec![address_to_whitelist.into(), Felt::ONE],
    };

    let res = account.execute_v3(vec![call]).send().await.expect("whitelist tx failed");

    katana_utils::TxWaiter::new(res.transaction_hash, &node.starknet_rpc_client()).await.unwrap();

    println!("Whitelisted VRF account {address_to_whitelist} on forwarder");
}
