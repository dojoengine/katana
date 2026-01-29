use crate::{create_app, state::AppState, Args};
use anyhow::{anyhow, Result};
use axum_test::TestServer;
use cainome_cairo_serde::{ClassHash, ContractAddress};
use cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;
use cairo_lang_starknet_classes::contract_class::ContractClass;
use dojo_utils::TransactionWaiter;
use katana_runner::RunnerCtx;
use num::FromPrimitive;
use starknet::{
    accounts::{Account, SingleOwnerAccount},
    core::{
        types::{
            contract::{CompiledClass, SierraClass},
            Call, FlattenedSierraClass,
        },
        utils::get_udc_deployed_address,
    },
    macros::{felt, selector},
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
    signers::LocalWallet,
};
use starknet_crypto::Felt;
use std::{fs::File, path::PathBuf, sync::Arc};

pub async fn new_test_server(args: &Args) -> TestServer {
    let app_state = AppState::from_args(args).await;
    let app = create_app(app_state).await;

    TestServer::builder()
        .expect_success_by_default()
        .mock_transport()
        .build(app)
        .unwrap()
}

pub const UDC_ADDRESS: Felt =
    felt!("0x41a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf");

pub const STRK_ADDRESS: Felt =
    felt!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

pub const ACCOUNT_MOCK_ARTIFACT: &str =
    "../target/dev/cartridge_vrf_AccountMock.contract_class.json";
pub const ACCOUNT_MOCK_PUBLIC_KEY: Felt =
    felt!("0x4c339f18b9d1b95b64a6d378abd1480b2e0d5d5bd33cd0828cbce4d65c27284");
pub const ACCOUNT_MOCK_PRIVATE_KEY: Felt =
    felt!("0x1c9053c053edf324aec366a34c6901b1095b07af69495bffec7d7fe21effb1b");

pub const VRF_ACCOUNT_ARTIFACT: &str = "../target/dev/cartridge_vrf_VrfAccount.contract_class.json";
pub const VRF_CONSUMER_ARTIFACT: &str =
    "../target/dev/cartridge_vrf_VrfConsumer.contract_class.json";

pub const VRF_ACCOUNT_PRIVATE_KEY: &str = "0x111";
pub const VRF_ACCOUNT_PUBLIC_KEY: Felt =
    felt!("0x14584bef56c98fbb91aba84c20724937d5b5d2d6e5a49b60e6c3a19696fad5f");

pub const VRF_SECRET_KEY: u64 = 420;
pub const VRF_PUBLIC_KEY: [Felt; 2] = [
    felt!("0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4"),
    felt!("0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232"),
];

pub type StarknetAccount = SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>;

pub async fn declare(sequencer: &RunnerCtx, artifact: &str) -> ClassHash {
    let account = sequencer.account(0);

    let (sierra_class, casm_class_hash) =
        prepare_contract_declaration_params(&PathBuf::from(artifact)).unwrap();

    let declare_result = account
        .declare_v3(Arc::new(sierra_class), casm_class_hash)
        .send()
        .await
        .unwrap();

    let class_hash = declare_result.class_hash;

    let _declare_receipt =
        TransactionWaiter::new(declare_result.transaction_hash, sequencer.provider())
            .await
            .unwrap();

    class_hash.into()
}
pub async fn declare_and_deploy(
    sequencer: &RunnerCtx,
    artifact: &str,
    constructor_calldata: Vec<Felt>,
) -> (ContractAddress, ClassHash) {
    let account = sequencer.account(0);

    let class_hash = declare(sequencer, artifact).await;

    let mut calldata = vec![
        class_hash.0,
        felt!("0x0"),                                          // salt
        felt!("0x0"),                                          // unique
        Felt::from_usize(constructor_calldata.len()).unwrap(), // calldata len
    ];

    calldata.extend(constructor_calldata.clone()); // calldata

    let deploy_result = account
        .execute_v3(vec![Call {
            to: UDC_ADDRESS,
            selector: selector!("deployContract"),
            calldata,
        }])
        .send()
        .await
        .unwrap();

    let _deploy_receipt =
        TransactionWaiter::new(deploy_result.transaction_hash, sequencer.provider())
            .await
            .unwrap();

    let contract_address = get_udc_deployed_address(
        felt!("0x0"), // salt
        class_hash.0,
        &starknet::core::utils::UdcUniqueness::NotUnique,
        &constructor_calldata,
    );

    (contract_address.into(), class_hash)
}

// from katana

pub fn prepare_contract_declaration_params(
    artifact_path: &PathBuf,
) -> Result<(FlattenedSierraClass, Felt)> {
    let flattened_class = get_flattened_class(artifact_path)
        .map_err(|e| anyhow!("error flattening the contract class: {e}"))?;
    let compiled_class_hash = get_compiled_class_hash(artifact_path)
        .map_err(|e| anyhow!("error computing compiled class hash: {e}"))?;
    Ok((flattened_class, compiled_class_hash))
}

fn get_flattened_class(artifact_path: &PathBuf) -> Result<FlattenedSierraClass> {
    let file = File::open(artifact_path)?;
    let contract_artifact: SierraClass = serde_json::from_reader(&file)?;
    Ok(contract_artifact.flatten()?)
}

fn get_compiled_class_hash(artifact_path: &PathBuf) -> Result<Felt> {
    let file = File::open(artifact_path)?;
    let casm_contract_class: ContractClass = serde_json::from_reader(file)?;
    let casm_contract =
        CasmContractClass::from_contract_class(casm_contract_class, true, usize::MAX)
            .map_err(|e| anyhow!("CasmContractClass from ContractClass error: {e}"))?;
    let res = serde_json::to_string_pretty(&casm_contract)?;
    let compiled_class: CompiledClass = serde_json::from_str(&res)?;
    Ok(compiled_class.class_hash()?)
}
