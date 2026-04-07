#![cfg(feature = "cartridge")]

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use cainome::rs::abigen_legacy;
use cartridge::api::GetAccountCalldataResponse;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::server::ServerBuilder;
use katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_primitives::execution::Call;
use katana_primitives::{address, felt, ContractAddress, Felt};
use katana_rpc_api::cartridge::CartridgeApiClient;
use katana_rpc_api::paymaster::PaymasterApiServer;
use katana_rpc_api::txpool::TxPoolApiClient;
use katana_rpc_types::txpool::{TxPoolContent, TxPoolStatus};
use katana_rpc_types::{OutsideExecution, OutsideExecutionV2};
use katana_utils::node::test_config_with_controllers;
use katana_utils::TestNode;
use parking_lot::Mutex;
use paymaster_rpc::{
    BuildTransactionRequest, BuildTransactionResponse, ExecuteRawRequest, ExecuteRawResponse,
    ExecuteRawTransactionParameters, ExecuteRequest, ExecuteResponse, RawInvokeParameters,
    TokenPrice,
};
use serde::Deserialize;
use starknet::accounts::{Account, AccountError, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::StarknetError;
use starknet::providers::ProviderError;
use starknet::signers::{LocalWallet, SigningKey};
use tokio::net::TcpListener;
use url::Url;

mod common;

abigen_legacy!(EthTokenContract, "crates/contracts/build/legacy/erc20.json", derives(Clone));

const VALID_CONTROLLER_ADDRESS: ContractAddress =
    address!("0x48e13ef7ab79637afd38a4b022862a7e6f3fd934f194c435d7e7b17bac06715");

/// The Controller middleware should add a deploy transaction for an undeployed Controller account.
#[tokio::test]
async fn controller_account_undeployed_should_deploy() {
    let sender = VALID_CONTROLLER_ADDRESS;
    let outside_execution = get_outside_execution();
    let signature = vec![Felt::ZERO, Felt::ZERO];

    let (cartridge_api_url, api_state) = start_mock_cartridge_api().await;
    let (paymaster_url, paymaster_state) = start_mock_paymaster().await;

    let config = cartridge_test_config(cartridge_api_url, paymaster_url);

    let node = TestNode::new_with_config(config).await;
    let rpc_client = node.rpc_http_client();

    let controller_deployer = node
        .handle()
        .node()
        .config()
        .paymaster
        .as_ref()
        .unwrap()
        .cartridge_api
        .as_ref()
        .unwrap()
        .controller_deployer_address;

    rpc_client
        .add_execute_outside_transaction(sender, outside_execution.clone(), signature.clone(), None)
        .await
        .unwrap();

    let api_requests = api_state.received_requests.lock();
    assert_eq!(api_requests.len(), 1, "Cartridge API should have been queried once");

    let status: TxPoolStatus = rpc_client.txpool_status().await.unwrap();
    assert_eq!(status.pending, 1, "pool should contain 1 deploy transaction");

    let content: TxPoolContent = rpc_client.txpool_content_from(controller_deployer).await.unwrap();
    assert_eq!(content.pending.len(), 1, "deploy tx should be from the deployer");

    let paymaster_requests = paymaster_state.execute_raw_transaction_requests.lock();
    let request = paymaster_requests.get(&sender).expect("tx should be forwarded to paymaster");
    assert_eq!(request.len(), 1, "should have one request forwarded to paymaster");
}

/// The Controller middleware shouldn't add a deploy transaction for an already deployed account
/// (regardless if the account is a Controller or not).
///
/// The execute outside transaction request would be simply fall through to the Cartridge API and
/// forwarded to the paymaster.
#[tokio::test]
async fn account_deployed_should_not_deploy() {
    let (cartridge_api_url, mock_api_state) = start_mock_cartridge_api().await;
    let (paymaster_url, mock_paymaster_state) = start_mock_paymaster().await;

    let config = cartridge_test_config(cartridge_api_url, paymaster_url);

    let node = TestNode::new_with_config(config).await;
    let rpc_client = node.rpc_http_client();

    let sender = node.account(); // pre-deployed account
    let sender = ContractAddress::from(sender.address());
    let outside_execution = get_outside_execution();

    rpc_client
        .add_execute_outside_transaction(sender, outside_execution, Vec::new(), None)
        .await
        .unwrap();

    let api_requests = mock_api_state.received_requests.lock();
    assert!(!api_requests.contains(&sender), "no api query bcs the account is deployed");

    let status: TxPoolStatus = rpc_client.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0, "pool should not contain a deploy transaction");

    let paymaster_requests = mock_paymaster_state.execute_raw_transaction_requests.lock();
    let request = paymaster_requests.get(&sender).expect("tx should be forwarded to paymaster");
    assert_eq!(request.len(), 1, "should have one request forwarded to paymaster");
}

/// The Controller middleware shouldn't add a deploy transaction for an undeployed non-Controller
/// account.
///
/// The execute outside transaction request would be simply fall through to the Cartridge API and
/// forwarded to the paymaster.
#[tokio::test]
async fn non_controller_account_undeployed_should_not_deploy() {
    let (cartridge_api_url, mock_api_state) = start_mock_cartridge_api().await;
    let (paymaster_url, mock_paymaster_state) = start_mock_paymaster().await;

    let config = cartridge_test_config(cartridge_api_url, paymaster_url);

    let node = TestNode::new_with_config(config).await;
    let rpc_client = node.rpc_http_client();

    let sender = address!("0xdeadbeef");
    let outside_execution = get_outside_execution();

    rpc_client
        .add_execute_outside_transaction(sender, outside_execution.clone(), Vec::new(), None)
        .await
        .unwrap();

    let api_requests = mock_api_state.received_requests.lock();
    assert!(api_requests.contains(&sender), "should query api bcs account is undeployed");

    let status: TxPoolStatus = rpc_client.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0, "no deploy tx for non-Controller account");

    let paymaster_requests = mock_paymaster_state.execute_raw_transaction_requests.lock();
    let request = paymaster_requests.get(&sender).expect("tx should be forwarded to paymaster");
    assert_eq!(request.len(), 1, "should have one request forwarded to paymaster");
}

#[tokio::test]
async fn estimate_fee_controller_account_undeployed_should_deploy() {
    let controller_address = VALID_CONTROLLER_ADDRESS;

    let (cartridge_api_url, mock_api_state) = start_mock_cartridge_api().await;
    let (paymaster_url, ..) = start_mock_paymaster().await;

    let config = cartridge_test_config(cartridge_api_url, paymaster_url);
    let node = TestNode::new_with_config(config).await;

    // Use an undeployed non-Controller account to execute the request
    let account = SingleOwnerAccount::new(
        node.starknet_provider(),
        // account validation is disabled on the test node
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(Felt::ZERO)),
        controller_address.into(),
        node.backend().chain_spec.id().into(),
        ExecutionEncoding::New,
    );

    let contract = EthTokenContract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    let _ = contract
	    .transfer(&Felt::ONE, &Uint256 { low: Felt::ZERO, high: Felt::ZERO })
        .nonce(Felt::ONE) // to avoid nonce query internally
        .estimate_fee()
        .await
        .unwrap();

    // Cartridge API should still be queried on estimate fee
    let api_requests = mock_api_state.received_requests.lock();
    assert!(api_requests.contains(&controller_address), "Cartridge API should be queried once");
}

/// Estimate fee works normally for an account that is already deployed regardless if the account
/// is a Controller or not. It's treated like a normal estimate fee request.
#[tokio::test]
async fn estimate_fee_account_deployed_works_normally() {
    let (cartridge_api_url, mock_api_state) = start_mock_cartridge_api().await;
    let (paymaster_url, ..) = start_mock_paymaster().await;

    let config = cartridge_test_config(cartridge_api_url, paymaster_url);
    let node = TestNode::new_with_config(config).await;

    let account = node.account(); // pre-deployed account
    let sender = ContractAddress::from(account.address());

    let account = SingleOwnerAccount::new(
        node.starknet_provider(),
        // account validation is disabled on the test node
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(Felt::ZERO)),
        sender.into(),
        node.backend().chain_spec.id().into(),
        ExecutionEncoding::New,
    );

    let contract = EthTokenContract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    contract
        .transfer(&Felt::ONE, &Uint256 { low: Felt::ZERO, high: Felt::ZERO })
        .estimate_fee()
        .await
        .unwrap();

    // Cartridge API should still be queried on estimate fee
    let api_requests = mock_api_state.received_requests.lock();
    assert!(api_requests.contains(&sender), "Cartridge API should be queried once");
}

#[tokio::test]
async fn estimate_fee_non_controller_account_undeployed_should_not_deploy() {
    let (cartridge_api_url, mock_api_state) = start_mock_cartridge_api().await;
    let (paymaster_url, ..) = start_mock_paymaster().await;

    let config = cartridge_test_config(cartridge_api_url, paymaster_url);
    let node = TestNode::new_with_config(config).await;

    let non_controller_address = address!("0xdeadbeef");

    // Use an undeployed non-Controller account to execute the request
    let account = SingleOwnerAccount::new(
        node.starknet_provider(),
        // account validation is disabled on the test node
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(Felt::ZERO)),
        non_controller_address.into(),
        node.backend().chain_spec.id().into(),
        ExecutionEncoding::New,
    );

    let contract = EthTokenContract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    let err = contract
    	.transfer(&Felt::ONE, &Uint256 { low: Felt::ZERO, high: Felt::ZERO })
        .nonce(Felt::ONE) // to avoid nonce query internally
        .estimate_fee()
        .await
        .unwrap_err();

    // The request should fail because the account is undeployed
    assert_matches::assert_matches!(
        err,
        AccountError::Provider(ProviderError::StarknetError(StarknetError::ContractNotFound))
    );

    // Cartridge API should still be queried on estimate fee
    let api_requests = mock_api_state.received_requests.lock();
    assert!(api_requests.contains(&non_controller_address), "Cartridge API should be queried once");
}

fn get_outside_execution() -> OutsideExecution {
    OutsideExecution::V2(OutsideExecutionV2 {
        caller: address!("0x414e595f43414c4c4552"),
        nonce: Felt::ONE,
        execute_after: 0,
        execute_before: u64::MAX,
        calls: vec![Call {
            contract_address: ContractAddress::ONE,
            entry_point_selector: felt!("0x2"),
            calldata: vec![felt!("0x3")],
        }],
    })
}

fn cartridge_test_config(
    cartridge_api_url: url::Url,
    paymaster_url: url::Url,
) -> katana_sequencer_node::config::Config {
    use katana_sequencer_node::config::paymaster::{CartridgeApiConfig, PaymasterConfig};

    let mut config = test_config_with_controllers();
    config.dev.account_validation = false;
    config.sequencing.no_mining = true;

    let (deployer_address, deployer_account) =
        config.chain.genesis().accounts().next().expect("must have genesis accounts");
    let deployer_private_key = deployer_account.private_key().expect("must have private key");

    config.paymaster = Some(PaymasterConfig {
        url: paymaster_url,
        api_key: None,
        cartridge_api: Some(CartridgeApiConfig {
            cartridge_api_url,
            controller_deployer_address: *deployer_address,
            controller_deployer_private_key: deployer_private_key,
            #[cfg(feature = "vrf")]
            vrf: None,
        }),
    });

    config
}

#[derive(Clone, Default)]
struct MockCartridgeApiState {
    received_requests: Arc<Mutex<Vec<ContractAddress>>>,
}

async fn start_mock_cartridge_api() -> (url::Url, MockCartridgeApiState) {
    #[derive(Debug, Deserialize)]
    struct GetAccountCalldataBody {
        address: ContractAddress,
    }

    async fn get_account_calldata_handler(
        State(state): State<MockCartridgeApiState>,
        Json(GetAccountCalldataBody { address }): Json<GetAccountCalldataBody>,
    ) -> Response {
        state.received_requests.lock().push(address);

        if address == VALID_CONTROLLER_ADDRESS {
            Json(GetAccountCalldataResponse {
                address: VALID_CONTROLLER_ADDRESS,
                username: "testuser".to_string(),
                constructor_calldata: vec![
                    felt!("0x24a9edbfa7082accfceabf6a92d7160086f346d622f28741bf1c651c412c9ab"),
                    felt!("0x676c69686d"),
                    felt!("0x0"),
                    felt!("0x1e"),
                    felt!("0x0"),
                    felt!("0x4"),
                    felt!("0x16"),
                    felt!("0x68"),
                    felt!("0x74"),
                    felt!("0x74"),
                    felt!("0x70"),
                    felt!("0x73"),
                    felt!("0x3a"),
                    felt!("0x2f"),
                    felt!("0x2f"),
                    felt!("0x78"),
                    felt!("0x2e"),
                    felt!("0x63"),
                    felt!("0x61"),
                    felt!("0x72"),
                    felt!("0x74"),
                    felt!("0x72"),
                    felt!("0x69"),
                    felt!("0x64"),
                    felt!("0x67"),
                    felt!("0x65"),
                    felt!("0x2e"),
                    felt!("0x67"),
                    felt!("0x67"),
                    felt!("0x9d0aec9905466c9adf79584fa75fed3"),
                    felt!("0x20a97ec3f8efbc2aca0cf7cabb420b4a"),
                    felt!("0x30910fae3f3451a26071c3afc453425e"),
                    felt!("0xa4e54fa48a6c3f34444687c2552b157f"),
                    felt!("0x1"),
                ],
            })
            .into_response()
        } else {
            "Address not found".into_response()
        }
    }

    let state = MockCartridgeApiState::default();

    let app = Router::new()
        .route("/accounts/calldata", post(get_account_calldata_handler))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = Url::parse(&format!("http://{addr}")).unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (url, state)
}

async fn start_mock_paymaster() -> (Url, MockPaymaster) {
    let mock = MockPaymaster { execute_raw_transaction_requests: Default::default() };

    let server = ServerBuilder::default().build("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let handle = server.start(mock.clone().into_rpc());
    std::mem::forget(handle);

    (url::Url::parse(&format!("http://{addr}")).unwrap(), mock)
}

#[derive(Clone)]
struct MockPaymaster {
    // track execute_raw_transaction requests
    execute_raw_transaction_requests:
        Arc<Mutex<HashMap<ContractAddress, Vec<RawInvokeParameters>>>>,
}

#[async_trait]
impl PaymasterApiServer for MockPaymaster {
    async fn health(&self) -> RpcResult<bool> {
        Ok(true)
    }

    async fn is_available(&self) -> RpcResult<bool> {
        Ok(true)
    }

    async fn build_transaction(
        &self,
        req: BuildTransactionRequest,
    ) -> RpcResult<BuildTransactionResponse> {
        let _ = req;
        unimplemented!()
    }

    async fn execute_transaction(&self, req: ExecuteRequest) -> RpcResult<ExecuteResponse> {
        let _ = req;
        unimplemented!()
    }

    async fn execute_raw_transaction(
        &self,
        req: ExecuteRawRequest,
    ) -> RpcResult<ExecuteRawResponse> {
        match req.transaction {
            ExecuteRawTransactionParameters::RawInvoke { invoke } => {
                let sender_address = invoke.user_address;
                self.execute_raw_transaction_requests
                    .lock()
                    .entry(sender_address.into())
                    .or_default()
                    .push(invoke);
            }
        }

        Ok(ExecuteRawResponse { transaction_hash: felt!("0xcafe"), tracking_id: Felt::ZERO })
    }

    async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>> {
        Ok(vec![])
    }
}
