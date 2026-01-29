use crate::{
    routes::outside_execution::{
        build_signed_outside_execution_v2,
        context::RequestContext,
        types::{OutsideExecution, OutsideExecutionV2, SignedOutsideExecution},
        OutsideExecutionRequest, OutsideExecutionResult, ANY_CALLER,
    },
    tests::setup::{
        declare_and_deploy, new_test_server, ACCOUNT_MOCK_ARTIFACT, ACCOUNT_MOCK_PRIVATE_KEY,
        ACCOUNT_MOCK_PUBLIC_KEY, STRK_ADDRESS, VRF_ACCOUNT_ARTIFACT, VRF_ACCOUNT_PRIVATE_KEY,
        VRF_ACCOUNT_PUBLIC_KEY, VRF_CONSUMER_ARTIFACT, VRF_PUBLIC_KEY, VRF_SECRET_KEY,
    },
    Args,
};
use dojo_utils::TransactionWaiter;
use katana_runner::RunnerCtx;
use num::FromPrimitive;
use serde_json::json;
use starknet::{
    accounts::{Account, SingleOwnerAccount},
    core::types::{BlockId, Call, FunctionCall},
    macros::{felt, selector},
    providers::Provider,
    signers::{LocalWallet, SigningKey},
};
use starknet_crypto::Felt;

#[tokio::test(flavor = "multi_thread")]
#[katana_runner::test(accounts = 10, chain_id = Felt::from_hex_unchecked("0x57505f4b4154414e41"))] // WP_KATANA
async fn test_outside_execution(sequencer: &RunnerCtx) {
    let chain_id = sequencer.provider().chain_id().await.unwrap();
    let account = sequencer.account(0);

    let (vrf_account_address, _) = declare_and_deploy(
        sequencer,
        VRF_ACCOUNT_ARTIFACT,
        vec![VRF_ACCOUNT_PUBLIC_KEY],
    )
    .await;

    let vrf_signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(
        Felt::from_hex(VRF_ACCOUNT_PRIVATE_KEY).unwrap(),
    ));
    let vrf_account = SingleOwnerAccount::new(
        sequencer.provider(),
        vrf_signer,
        vrf_account_address.0,
        chain_id,
        starknet::accounts::ExecutionEncoding::New,
    );

    // transfer strk to vrf_account
    let transfer_tx_result = account
        .execute_v3(vec![Call {
            to: STRK_ADDRESS,
            selector: selector!("transfer"),
            calldata: vec![
                vrf_account_address.0,
                Felt::from_u128(10 * 10_u128.pow(18)).unwrap(),
                Felt::ZERO,
            ],
        }])
        .send()
        .await
        .unwrap();

    TransactionWaiter::new(transfer_tx_result.transaction_hash, sequencer.provider())
        .await
        .unwrap();

    // set_vrf_public_key
    let set_vrf_public_key_tx_result = vrf_account
        .execute_v3(vec![Call {
            to: vrf_account_address.0,
            selector: selector!("set_vrf_public_key"),
            calldata: VRF_PUBLIC_KEY.into(),
        }])
        .send()
        .await
        .unwrap();

    TransactionWaiter::new(
        set_vrf_public_key_tx_result.transaction_hash,
        sequencer.provider(),
    )
    .await
    .unwrap();

    let (consumer_address, _) = declare_and_deploy(
        sequencer,
        VRF_CONSUMER_ARTIFACT,
        vec![vrf_account_address.0],
    )
    .await;

    // MUST USE ACCOUNT SUPPORTING OUTSIDE_EXECUTION
    let (user_account_address, _) = declare_and_deploy(
        sequencer,
        ACCOUNT_MOCK_ARTIFACT,
        vec![ACCOUNT_MOCK_PUBLIC_KEY],
    )
    .await;

    let user_account_signer =
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_MOCK_PRIVATE_KEY));
    let user_account = SingleOwnerAccount::new(
        sequencer.provider(),
        user_account_signer.clone(),
        user_account_address.0,
        chain_id,
        starknet::accounts::ExecutionEncoding::New,
    );

    let user_calls = vec![
        Call {
            to: vrf_account_address.0,
            selector: selector!("request_random"),
            calldata: vec![
                consumer_address.0,
                felt!("0x0"), // Source::Nonce
                user_account.address(),
            ],
        }
        .into(),
        Call {
            to: consumer_address.0,
            selector: selector!("dice"),
            calldata: vec![],
        }
        .into(),
    ];

    let signed_outside_execution = build_signed_outside_execution_v2(
        user_account.address(),
        user_account_signer,
        chain_id,
        user_calls,
    )
    .await;

    // println!("signed_outisde_execution: {:?}", signed_outside_execution);

    let args = Args::default()
        .with_account_address(&vrf_account_address.0.to_hex_string())
        .with_account_private_key(VRF_ACCOUNT_PRIVATE_KEY)
        .with_secret_key(VRF_SECRET_KEY);

    let server = new_test_server(&args).await;

    let signed_outisde_execution_request_json = serde_json::to_value(&OutsideExecutionRequest {
        request: signed_outside_execution,
        context: RequestContext {
            chain_id: "WP_KATANA".into(),
            rpc_url: Option::Some(sequencer.url().to_string()),
        },
    })
    .unwrap();

    let response = server
        .post("/outside_execution")
        .json(&signed_outisde_execution_request_json)
        .await;

    let outside_execution_result = response.json::<OutsideExecutionResult>();
    let final_outside_execution = outside_execution_result.result;
    let execution_call = final_outside_execution.build_execute_from_outside_call();

    let executor_account = sequencer.account(2);

    let _dice_value = sequencer
        .provider()
        .call(
            FunctionCall {
                contract_address: consumer_address.0,
                entry_point_selector: selector!("get_dice_value"),
                calldata: vec![],
            },
            BlockId::Tag(starknet::core::types::BlockTag::PreConfirmed),
        )
        .await
        .unwrap();

    let execute_result = executor_account
        .execute_v3(vec![execution_call.into()])
        .send()
        .await
        .unwrap();

    let _execute_receipt =
        TransactionWaiter::new(execute_result.transaction_hash, sequencer.provider())
            .await
            .unwrap();

    let dice_value = sequencer
        .provider()
        .call(
            FunctionCall {
                contract_address: consumer_address.0,
                entry_point_selector: selector!("get_dice_value"),
                calldata: vec![],
            },
            BlockId::Tag(starknet::core::types::BlockTag::PreConfirmed),
        )
        .await
        .unwrap();

    println!("dice_value_after: {dice_value:?}");
    assert!(dice_value[0] == felt!("0x4"), "dice should be 6")
}

pub fn mock_signed_outside_execution() -> SignedOutsideExecution {
    SignedOutsideExecution {
        address: felt!("0x123"),
        outside_execution: OutsideExecution::V2(OutsideExecutionV2 {
            caller: ANY_CALLER,
            calls: vec![],
            execute_after: 0,
            execute_before: 0,
            nonce: felt!("0x0"),
        }),
        signature: vec![],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_with_bad_chain_id() {
    let args = Args::default()
        .with_account_private_key(VRF_ACCOUNT_PRIVATE_KEY)
        .with_secret_key(VRF_SECRET_KEY);

    let server = new_test_server(&args).await;

    let signed_outisde_execution_request_json = serde_json::to_value(&OutsideExecutionRequest {
        request: mock_signed_outside_execution(),
        context: RequestContext {
            chain_id: "WP_KATANA_THIS_IS_TOO_LONG_FOR_SHORT_STRING".into(),
            rpc_url: Option::None,
        },
    })
    .unwrap();

    server
        .post("/outside_execution")
        .json(&signed_outisde_execution_request_json)
        .expect_failure()
        .await
        .assert_status_not_found();

    let _ = server.get("/").expect_success();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_with_bad_rpc_url() {
    let args = Args::default()
        .with_account_private_key(VRF_ACCOUNT_PRIVATE_KEY)
        .with_secret_key(VRF_SECRET_KEY);

    let server = new_test_server(&args).await;

    let signed_outisde_execution_request_json = serde_json::to_value(&OutsideExecutionRequest {
        request: mock_signed_outside_execution(),
        context: RequestContext {
            chain_id: "WP_KATANA_".into(),
            rpc_url: Option::Some("not_a_rpc_url".into()),
        },
    })
    .unwrap();

    server
        .post("/outside_execution")
        .json(&signed_outisde_execution_request_json)
        .expect_failure()
        .await
        .assert_status_not_found();

    let _ = server.get("/").expect_success();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_with_malformed_json() {
    let args = Args::default()
        .with_account_private_key(VRF_ACCOUNT_PRIVATE_KEY)
        .with_secret_key(VRF_SECRET_KEY);

    let server = new_test_server(&args).await;

    let signed_outisde_execution_request_json = json!({
        "request": "invalid_json",
        "context": "invalid_json",
    });

    server
        .post("/outside_execution")
        .json(&signed_outisde_execution_request_json)
        .expect_failure()
        .await
        .assert_status_unprocessable_entity();

    let _ = server.get("/").expect_success();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_with_invalid_json_value() {
    let args = Args::default()
        .with_account_private_key(VRF_ACCOUNT_PRIVATE_KEY)
        .with_secret_key(VRF_SECRET_KEY);

    let server = new_test_server(&args).await;

    let signed_outisde_execution_request_json = json!({
      "request": {
        "address": "0x111",
        "outside_execution": {
          "V3": {
            "caller": "0x414e595f43414c4c4552",
            "calls": [
              {
                "calldata": ["0x111", "zzzzzzzz", "0x222"],
                "selector": "0x12a5a2e008479001f8f1a5f6c61ab6536d5ce46571fcdc0c9300dca0a9e532f",
                "to": "0x888"
              },
              {
                "calldata": [],
                "selector": "0x1f9ca87172ecd8343d776bdd6024a4028f5596c76320882abd93e3bd1c724eb",
                "to": "0x111"
              }
            ],
            "execute_after": "0x0",
            "execute_before": "0xb2d05e00",
            "nonce": [
              "0x564b73282b2fb5f201cf2070bf0ca2526871cb7daa06e0e805521ef5d907b33",
              "0xa"
            ]
          }
        },
        "signature": ["0x12345", "0x67890"]
      },
      "context": { "chain_id": "WP_KATANA", "rpc_url": "http://localhost:5050" }
    }
    );

    server
        .post("/outside_execution")
        .json(&signed_outisde_execution_request_json)
        .expect_failure()
        .await
        .assert_status_unprocessable_entity();

    let _ = server.get("/").expect_success();
}
