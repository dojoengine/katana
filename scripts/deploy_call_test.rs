//! Helper script that declares & deploys the CallTest contract on mainnet + fork nodes.
//! Set env vars if needed:
//!   MAINNET_RPC=http://127.0.0.1:5050
//!   FORK_RPC=http://127.0.0.1:5051
//!   DEPLOY_SALT=0x1234567890abcdef

use std::{fmt::Display, sync::Arc, time::Duration};
use starknet::accounts::Account;
use katana_primitives::class::{ContractClass, SierraContractClass};
use starknet::{
    accounts::{ConnectedAccount, ExecutionEncoding, SingleOwnerAccount},
    core::types::{
        contract::SierraClass as CoreSierraClass, BlockId, BlockTag, Felt, FlattenedSierraClass,
    },
    providers::{
        jsonrpc::{HttpTransport, JsonRpcClient},
        Provider,
    },
    signers::{LocalWallet, SigningKey},
};
use starknet_contract::ContractFactory;
use tokio::time::sleep;
use url::Url;

const PREFUNDED_PRIVATE_KEY: &str =
    "0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912";
const PREFUNDED_ACCOUNT_ADDRESS: &str =
    "0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec";
const DEFAULT_DEPLOY_SALT: Felt = Felt::from_hex_unchecked("0x1234567890abcdef");
const CALL_TEST_ARTIFACT: &[u8] =
    include_bytes!("../crates/contracts/build/katana_test_contracts_CallTest.contract_class.json");

type RpcProvider = Arc<JsonRpcClient<HttpTransport>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mainnet_rpc =
        std::env::var("MAINNET_RPC").unwrap_or_else(|_| "http://127.0.0.1:5050".to_string());
    let fork_rpc =
        std::env::var("FORK_RPC").unwrap_or_else(|_| "http://127.0.0.1:5051".to_string());
    let deploy_salt = std::env::var("DEPLOY_SALT")
        .map(|value| Felt::from_hex(&value))
        .unwrap_or(Ok(DEFAULT_DEPLOY_SALT))?;

    section("Configuration");
    info("Mainnet RPC", &mainnet_rpc);
    info("Fork RPC", &fork_rpc);
    info("Deploy salt", format!("{deploy_salt:#x}"));

    let mainnet_provider = build_provider(&mainnet_rpc)?;
    let fork_provider = build_provider(&fork_rpc)?;

    let account_address = Felt::from_hex(PREFUNDED_ACCOUNT_ADDRESS)?;
    let private_key = Felt::from_hex(PREFUNDED_PRIVATE_KEY)?;
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(private_key));

    let mainnet_account =
        build_account(mainnet_provider.clone(), signer.clone(), account_address).await?;
    let fork_account = build_account(fork_provider.clone(), signer, account_address).await?;

    let artifacts = load_call_test_artifacts()?;
    section("CallTest artifacts");
    info("Class hash", format!("{:#x}", artifacts.class_hash));
    info("Compiled class hash", format!("{:#x}", artifacts.compiled_class_hash));

    section("Deployment");
    declare_and_deploy_call_test("Mainnet", &mainnet_account, &artifacts, deploy_salt).await?;
    declare_and_deploy_call_test("Fork", &fork_account, &artifacts, deploy_salt).await?;

    section("Summary");
    println!("✅ CallTest contract declared & deployed on both nodes");
    Ok(())
}

fn build_provider(rpc: &str) -> Result<RpcProvider, Box<dyn std::error::Error>> {
    let url = Url::parse(rpc)?;
    Ok(Arc::new(JsonRpcClient::new(HttpTransport::new(url))))
}

async fn build_account(
    provider: RpcProvider,
    wallet: LocalWallet,
    address: Felt,
) -> Result<SingleOwnerAccount<RpcProvider, LocalWallet>, Box<dyn std::error::Error>> {
    let chain_id = provider.chain_id().await?;
    Ok(SingleOwnerAccount::new(provider, wallet, address, chain_id, ExecutionEncoding::New))
}

struct TestContractArtifacts {
    flattened: Arc<FlattenedSierraClass>,
    compiled_class_hash: Felt,
    class_hash: Felt,
}

fn load_call_test_artifacts() -> Result<TestContractArtifacts, Box<dyn std::error::Error>> {
    let katana_sierra: SierraContractClass = serde_json::from_slice(CALL_TEST_ARTIFACT)?;
    let contract_class = ContractClass::Class(katana_sierra);
    let starknet_sierra: CoreSierraClass = serde_json::from_slice(CALL_TEST_ARTIFACT)?;
    let flattened = starknet_sierra.flatten()?;

    let compiled = contract_class.clone().compile()?;
    let compiled_hash = compiled.class_hash()?;
    let class_hash = contract_class.class_hash()?;

    Ok(TestContractArtifacts {
        flattened: Arc::new(flattened),
        compiled_class_hash: compiled_hash,
        class_hash,
    })
}

async fn declare_and_deploy_call_test(
    label: &str,
    account: &SingleOwnerAccount<RpcProvider, LocalWallet>,
    artifacts: &TestContractArtifacts,
    deploy_salt: Felt,
) -> Result<(), Box<dyn std::error::Error>> {
    if account
        .provider()
        .get_class(BlockId::Tag(BlockTag::Latest), artifacts.class_hash)
        .await
        .is_ok()
    {
        info(&format!("{label} declare"), "already declared");
    } else {
        info(&format!("{label} declare"), "submitting transaction");
        let declare_res = account
            .declare_v3(artifacts.flattened.clone(), artifacts.compiled_class_hash)
            .send()
            .await?;
        info(
            &format!("{label} declare tx"),
            format!("{:#x}", declare_res.transaction_hash),
        );
        sleep(Duration::from_secs(2)).await;
    }

    info(&format!("{label} deploy"), "submitting UDC deployment");
    let factory = ContractFactory::new(artifacts.class_hash, account.clone());
    let deployment = factory.deploy_v3(Vec::new(), deploy_salt, true);
    let deployed_address = deployment.deployed_address();
    let deploy_res = deployment.send().await?;
    info(
        &format!("{label} deploy tx"),
        format!("{:#x}", deploy_res.transaction_hash),
    );
    info(
        &format!("{label} contract"),
        format!("{:#x}", deployed_address),
    );
    sleep(Duration::from_secs(2)).await;

    Ok(())
}

fn section(title: &str) {
    println!("\n=== {title} ===");
}

fn info(label: &str, value: impl Display) {
    println!("{label:>24}: {value}");
}

